# `batch translate-local` 運用メモ

この文書は、`batch translate-local` で rejected / failed の残件を補完するときの挙動と判断基準をまとめたものです。

## 役割

`batch translate-local` は、OpenAI Batch の残件を同じ batch 状態の中で埋めるための処理です。

- 対象は `local_pending` の item だけです。
- `imported` / `local_imported` や、有効なキャッシュがある item は再翻訳しません。
- ここで解けない item は、同じ方法で何度も粘らず `local_exhausted` に回します。

通常の `translate` が「書籍全体を進める」処理なのに対し、`batch translate-local` は「安く回収できる残件だけ拾う」処理です。

## 基本手順

まず残件を `local_pending` に回します。

```powershell
cargo run -- batch reroute-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --remaining `
  --priority short-first
```

次に `local_pending` だけを処理します。

```powershell
cargo run --release -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider ollama `
  --model qwen3:14b `
  --glossary .\glossary.json
```

OpenAI の通常 API を使う場合も同じコマンド面ですが、API キーはコマンド引数ではなく環境変数で与えてください。

```powershell
$env:OPENAI_API_KEY = "..."
cargo run --release -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

## 進捗表示

進捗行は次の形です。

```text
ok12 err5 | t9 c3 | p10 c8C.xhtml
```

- `ok12`: 完了件数。`translated + cached`
- `err5`: エラー件数
- `t9`: 新規翻訳成功件数
- `c3`: 既存キャッシュで解決した件数
- `p10 c8C.xhtml`: 現在のページとファイル

完了時は次のように出ます。

```text
done: 12 completed, 5 errors | 9 translated, 3 cached
```

## 停止条件

### 認証・設定エラー

provider の認証や権限に問題がある場合は、その時点で処理全体を停止します。

例:

- `HTTP 401`
- `HTTP 403`
- `Incorrect API key provided`
- `invalid_api_key`

この場合は残件を回し続けません。まず API キーや権限を直してください。

### 課金だけ進む停滞

完了件数が増えないまま API request だけが増える状態は異常とみなし、停止します。

現在の条件:

- 完了件数の増加が 0
- 10 分以上継続
- その間に API requests が 20 件以上増加

停止時は終了コード `2` の recoverable error になります。

### 1 block 内の見切り

次のような block は、`local_exhausted` に回して同じ経路で繰り返しません。

- `prompt_leak`
- `missing_placeholder`
- `unchanged_source`
- `refusal_or_explanation`
- 1 block で request を 3 回以上使ってなお失敗したもの

## 失敗理由の見方

失敗理由は `batch/work_items.jsonl` の `last_error` に残ります。

`last_error` には、次の情報が入ります。

- 元の error chain
- `validation_reason=...`
- `suggested_action=...`
- request 予算超過時の補足
- stall 停止時の補足

例:

```text
translation validation failed after 4 attempt(s): translation validation failed: provider response still contains a long untranslated English segment | local request budget exceeded: 4 request(s) used (limit 3) | validation_reason=untranslated_segment | suggested_action=batch_retry_requests_or_try_another_provider
```

## エラーの記録先

epubicus では、失敗の種類によって記録先が分かれています。`batch translate-local` だけを見ればよい場合と、Batch 全体の流れを見ないといけない場合があるので、最初に記録先を切り分けてください。

### 1. `work_items.jsonl`

場所:

```text
<cache>\batch\work_items.jsonl
```

用途:

- batch の各 block の現在状態
- `last_error`
- `state`
- `updated_at`

ここが **最優先の一次情報** です。`batch translate-local` 中に起きた失敗は、まずここに残ります。

主に見る項目:

- `custom_id`
- `page_index`
- `block_index`
- `href`
- `state`
- `last_error`

`last_error` には、現在は次が追記されます。

- provider 呼び出し失敗の詳細
- `validation_reason=...`
- `suggested_action=...`
- request 予算超過
- stalled 停止時の補足

### 2. `rejected.jsonl`

場所:

```text
<cache>\batch\rejected.jsonl
```

用途:

- OpenAI Batch の output を import した時に、validation で弾いた item の一覧

これは **remote Batch import の拒否記録** です。`batch translate-local` 実行中の失敗ログではありません。

使いどころ:

- Batch 応答そのものの品質傾向を見る
- `rejected` がどんな理由で多いかを分類する

### 3. `errors.jsonl`

場所:

```text
<cache>\batch\errors.jsonl
```

用途:

- import 時に remote error として扱われた request の記録

これは **remote request failure の記録** です。validation failure ではなく、Batch 側の request 単位の失敗を見ます。

### 4. `retry_requests.jsonl`

場所:

```text
<cache>\batch\retry_requests.jsonl
```

用途:

- `batch retry-requests` が再投入候補として書き出した request 一覧

これはエラーログではなく、**次の remote 再試行の入力**です。

### 5. `recovery.jsonl` / `failed.jsonl`

場所:

```text
<cache>\recovery\<output>\recovery.jsonl
<cache>\recovery\<output>\failed.jsonl
```

用途:

- `translate`
- `recover`
- `scan-recovery`

で扱う復旧対象の記録です。batch そのものの state 管理とは別系統ですが、最終的に EPUB 側に未翻訳が残った場合はこちらが重要になります。

## `work_items.jsonl` の見方

1 行が 1 block です。重要なのは `state` と `last_error` の組です。

### 代表的な state

- `imported`
  - remote Batch import 済み
- `rejected`
  - remote Batch output が validation で拒否された
- `failed`
  - remote Batch request failure
- `local_pending`
  - local 補完待ち
- `local_imported`
  - local 補完成功
- `local_exhausted`
  - local 補完では見切った
- `skipped`
  - 原文維持を意図的に確定した

### `local_exhausted` の意味

`local_exhausted` は「失敗した」のではなく、**この local 補完経路ではこれ以上粘らない**という意味です。

この state になった item は:

- `batch translate-local` の通常対象から外れる
- `batch retry-requests` の既定対象には含まれる
- 別 provider / model、remote retry、`recover` の候補になる

### `skipped` の意味

`skipped` は、**その block を翻訳対象としては扱わず、原文維持で確定した**という意味です。

主な用途:

- 文献、参考文献、URL を含む書誌ブロック
- 翻訳より正確な原文保持を優先したい block

この state になった item は:

- `batch translate-local` の対象に戻らない
- `batch verify` では正常扱いになる
- `translate --partial-from-cache` でも recovery 対象にせず、そのまま EPUB に組み込まれる

## `last_error` の読み方

`last_error` は自由文ですが、今の実装では次の順で読むと判断しやすいです。

1. provider 呼び出しに失敗したのか
2. validation で落ちたのか
3. `validation_reason` は何か
4. `suggested_action` は何か
5. request 予算超過や stall 補足があるか

### 例 1: 認証エラー

```text
failed to call OpenAI after 1 attempt(s): OpenAI HTTP 401 Unauthorized: ...
```

意味:

- request 自体が通っていない
- key / 権限 / endpoint 設定を直す

次の対応:

- `fix_provider_auth`

### 例 2: 未翻訳英語が残る

```text
translation validation failed after 4 attempt(s): translation validation failed: provider response still contains a long untranslated English segment | validation_reason=untranslated_segment | suggested_action=retry_translation
```

意味:

- request 自体は通っている
- provider 応答の英語が十分に日本語化されていない

次の対応:

- 通常の再翻訳
- 別 provider / model
- 復旧ログ経由の補完

### 例 3: local では見切り

```text
... | local request budget exceeded: 4 request(s) used (limit 3) | validation_reason=untranslated_segment | suggested_action=batch_retry_requests_or_try_another_provider
```

意味:

- 同じ block に対して local 補完で十分に試した
- この経路ではもう粘らない

次の対応:

- `batch retry-requests`
- 別 provider / model

### 例 4: stall で停止

```text
... | batch translate-local stalled: 12 completed, 5 errors; no new completions for 10m 06s while API requests increased by 20; current item p10 b50 c8C.xhtml
```

意味:

- 停止の直接原因は stall guard
- ただし前半の failure reason が本来の原因

読み方:

- 文末の stalled だけを見ない
- その前の `validation_reason` と `suggested_action` を優先する

## recovery 記録との関係

通常翻訳と batch local で記録形式は完全には同じではありませんが、**読む観点は揃える**方針です。

### recovery record 側

`recovery.jsonl` の 1 record には、次が入ります。

- `reason`
- `error`
- `suggested_action`
- `page_no`
- `block_index`
- `href`
- `cache_key`
- `source_hash`

### batch local 側

`work_items.jsonl` の `last_error` に、同じ判断に必要な情報を寄せています。

- `validation_reason=...`
- `suggested_action=...`

そのため、将来的に人手で復旧判断するときは、

- batch state を見るなら `work_items.jsonl`
- EPUB 側の未翻訳を埋めるなら `recovery.jsonl`

という役割分担で考えると整理しやすいです。

## `suggested_action` の意味

- `fix_provider_auth`
  - API キーや権限を直してから再実行します。
- `retry_translation`
  - 通常の再翻訳系で対応する想定です。通常のローカル翻訳や `recover` の対象です。
- `retry_translation_or_inspect_inline`
  - プレースホルダ崩れです。再翻訳するか、タグ復元を意識して確認します。
- `batch_retry_requests_or_try_another_provider`
  - `batch translate-local` では見切った block です。`batch retry-requests` で再投入するか、別 provider / model を検討します。
- `inspect_manually`
  - 自動判定で方向を決めきれなかったものです。個別確認します。

## `untranslated_segment` の扱い

`untranslated_segment` は、通常のローカル翻訳でも recovery 側では `retry_translation` 系として扱っています。`batch translate-local` でも同じ考え方で扱います。

ただし、`batch translate-local` は残件処理なので、同じ block に長く粘りません。

- 少数回の retry は行う
- それでも英語が長く残るなら `local_exhausted`
- 次は `retry_requests`、別 provider / model、`recover` のいずれかへ回す

という流れです。

## 再開時の見方

`batch health` では次を確認します。

- `local_pending`: まだ local 補完対象
- `local_imported`: local 補完済み
- `local_exhausted`: local 補完では見切った item
- `skipped`: 原文維持で確定した item
- `effective remaining`: 実際にまだ埋まっていない件数

`local_exhausted` は `--remaining` の通常対象から外れます。再度同じ local 補完を流しても、そこには戻りません。
`skipped` は「残件」には数えません。

## リカバリー方法の選び方

### 1. まだ `local_pending` が残っている

そのまま `batch translate-local` を続けます。

```powershell
cargo run --release -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider ollama `
  --model qwen3:14b `
  --glossary .\glossary.json
```

### 2. `local_exhausted` が増えてきた

同じ local 補完経路ではなく、remote 再投入か別 provider を検討します。

```powershell
cargo run -- batch retry-requests .\book.epub `
  --cache-root .\.batch-openai-cache `
  --limit 100 `
  --priority failed-first
```

### 2a. 文献系を原文維持で確定したい

参考文献や URL 含みの block は、無理に再翻訳せず `skipped` として原文維持で閉じる方が自然な場合があります。

この場合は:

- `effective remaining` から外れる
- `batch verify` では正常扱い
- `translate --partial-from-cache` でも `Untranslated report` を増やさない

という扱いになります。

### 3. 最終 EPUB に未翻訳が残った

`translate --partial-from-cache` 後に `Recovery log:` が出たら、`recover` 系へ移ります。

```powershell
cargo run -- recover $log --provider ollama --model qwen3:14b
```

### 4. 出力 EPUB を外から検査して復旧したい

`scan-recovery` を使います。

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
```

## 実務上の見方

迷ったときは、次の順で見ます。

1. `batch health`
   - `effective remaining`
   - `local_pending`
   - `local_exhausted`
2. `work_items.jsonl`
   - 問題 block の `last_error`
3. 必要なら `rejected.jsonl` / `errors.jsonl`
4. EPUB 出力後の問題なら `recovery.jsonl`

この順にすると、

- batch 状態の問題か
- local 補完の見切りか
- 最終 EPUB の未翻訳か

を切り分けやすくなります。

## 次の一手

### local 補完の残りを続ける

```powershell
cargo run --release -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider ollama `
  --model qwen3:14b `
  --glossary .\glossary.json
```

### OpenAI Batch に再投入する

```powershell
cargo run -- batch retry-requests .\book.epub `
  --cache-root .\.batch-openai-cache `
  --limit 100 `
  --priority failed-first
```

### recovery log から別経路で埋める

```powershell
cargo run -- recover $log --provider ollama --model qwen3:14b
```

### キャッシュから EPUB を再生成する

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.batch-openai-cache `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```
