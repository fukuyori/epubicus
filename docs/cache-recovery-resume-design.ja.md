# キャッシュ・リカバリー・再開設計メモ

この文書は、EPUB 翻訳処理を「原則としていつでも中断・再開可能」にし、さらに途中で provider / model を変えても見通しよく継続できるようにするための設計メモです。

まだ実装しません。ここでは現状の整理、目標、CLI 案、段階的な実装順序を決めます。

## 目的

- 通常翻訳は、同じ入力 EPUB と同じ翻訳条件で再実行すれば、常にキャッシュから再開できる。
- 未翻訳や検証失敗が残った場合は、復旧対象だけを取り出して再試行できる。
- 復旧時は、途中で provider / model を変えても、元の出力 EPUB を完成させるためのキャッシュへ書き戻せる。
- 手動訳は provider を呼ばず、同じ復旧経路でキャッシュへ直接書き込める。
- 部分出力や再ビルドで、未完成の EPUB を不用意に完成版として上書きしない。

## 現状

現在のキャッシュディレクトリは、入力 EPUB の hash で決まります。

```text
<cache-root>\<input-hash>\
  manifest.json
  translations.jsonl
  recovery\<output-name>\
    untranslated.txt
    recovery.jsonl
    failed.jsonl
```

一方、各翻訳ブロックの cache key には次の情報が入ります。

- prompt version
- provider
- model
- style
- glossary subset
- source text

そのため、通常の `translate --partial-from-cache` は安全です。同じ provider / model / style / glossary なら確実に再利用でき、条件が違えば別物として扱います。

ただし、途中で model を変えると、同じ原文でも別 cache key になります。結果として、通常の `translate --partial-from-cache` では全 cache miss に見えやすくなります。

## 基本方針

### 0. スクリプトは少ない引数で実行できるようにする

運用スクリプトは、原則として 3 つ以内の引数で日常実行できる形にします。特別な指示が必要な場合でも 4 つまでに収めます。

そのため、よく使う値は次の場所へ寄せます。

- 入力 EPUB からの自動推定
- provider ごとの既定 cache root
- 入力 EPUB と同名の glossary 自動検出
- テンプレートスクリプト内の既定 model / concurrency
- `-NoRun` のような確認用 switch

スクリプトの役割は、長い `cargo run --release -- ...` を安全な定型操作に折りたたむことです。細かな CLI オプションをすべてスクリプト引数として露出しません。

### 1. 通常翻訳は厳密キーを維持する

`translate` の既定挙動は、今後も provider / model を含む厳密な cache key を使います。

理由:

- model を変えると訳文品質や文体が変わる。
- glossary や prompt version の違いを混ぜると再現性が落ちる。
- 完成 EPUB の一貫性を守るには、既定は保守的であるべき。

### 2. モデル変更は recovery 経由で行う

不足分を別 model で埋める標準経路は `recover` にします。

`recovery.jsonl` には、元の出力を完成させるための `cache_key` が記録されています。`recover` では、実際に呼ぶ provider / model を変えても、成功した訳文は recovery record の `cache_key` に書き戻します。

つまり、次のような扱いです。

```text
通常翻訳:
  deepseek-v4-flash の cache key で大半を翻訳

復旧:
  残った item だけ deepseek-v4-pro で翻訳
  ただし書き戻し先は recovery record の cache key

再ビルド:
  元の deepseek-v4-flash 条件の cache key で EPUB を組み立てる
```

この方針なら、完成 EPUB は「元の翻訳条件の出力を、復旧で補ったもの」として扱えます。

### 3. 互換キャッシュ再利用は明示 opt-in にする

通常 `translate` で model を変えたとき、別 model の既存訳を自動利用する機能は既定 ON にしません。

将来的に入れるなら、明示オプションにします。

```powershell
epubicus translate .\book.epub `
  --provider deepseek `
  --model deepseek-v4-pro `
  --reuse-compatible-cache
```

この場合の候補条件は、少なくとも次を一致させます。

- prompt version
- style
- glossary subset
- source text hash

provider / model は違ってもよい。ただし、出力サマリーには「互換キャッシュ流用」として件数を分けて表示します。

```text
Cache:
  hits: 1200
  compatible hits: 42
  misses: 3
```

## 推奨ワークフロー

### 通常の中断・再開

同じスクリプトを再実行します。

```powershell
.\scripts\translate-deepseek.ps1 .\book.epub
```

これは同じ provider / model / glossary で続行する前提です。

### 未翻訳が残った場合

まず同じ model で recovery します。

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek
```

同じ model で何度か試して改善しない場合、強い model に切り替えます。

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Model deepseek-v4-pro
```

少数だけ残る場合は、手動訳を直接キャッシュへ入れます。

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Manual .\book.manual.json
```

## CLI 設計案

### `recover`

既定方針:

- recovery record の `cache_key` へ書く。
- CLI で指定された provider / model は「復旧に使う実行 provider / model」。
- `--rebuild` は、元 record の provider / model / style を使って `translate --partial-from-cache` する。

追加したい表示:

```text
Recovery:
  cache identity: deepseek / deepseek-v4-flash
  execution model: deepseek / deepseek-v4-pro
  cache updated: 12
  already valid: 3
  unrecoverable: 0
```

この表示により、「別モデルで実行したが、元のキャッシュを完成させている」ことが分かります。

### `translate --partial-from-cache`

既定では厳密 cache key のみを使います。

将来オプション:

```text
--reuse-compatible-cache
```

意味:

- 現在の厳密 key が miss した場合だけ、同じ source identity の既存訳を探す。
- 見つかった訳文はその場で使う。
- ただし、現在の厳密 key に自動コピーするかどうかは別オプションに分ける。

追加候補:

```text
--promote-compatible-cache
```

意味:

- 互換キャッシュで使った訳文を、現在の厳密 key にも保存する。
- 混在を固定化するため、既定 OFF。

### `cache show`

cache 内に複数 provider / model の訳文が混ざる場合、内訳を見られるようにします。

```text
Translations:
  total: 1098
  by provider/model:
    deepseek/deepseek-v4-flash: 1081
    deepseek/deepseek-v4-pro: 17
```

ただし、現在の設計では別 model で復旧した訳文も元の cache key に書き戻すため、`CacheRecord.provider` / `CacheRecord.model` は「実際に生成した provider / model」を保持します。

## source identity

互換キャッシュを実装する場合は、厳密 key とは別に source identity が必要です。

候補:

```text
source_identity = hash(
  prompt_version,
  style,
  glossary_subset,
  source_text
)
```

provider / model は含めません。

`translations.jsonl` に将来フィールドを追加します。

```json
{
  "key": "strict-cache-key",
  "source_identity": "provider-model-independent-key",
  "translated": "...",
  "provider": "deepseek",
  "model": "deepseek-v4-pro",
  "at": "..."
}
```

後方互換:

- 古い record に `source_identity` がなくても読めるようにする。
- 必要なら読み込み時に `key` だけで厳密 lookup する。
- 互換 lookup は `source_identity` のある record だけ対象にする。

## 部分出力と上書き安全性

現在の `--partial-from-cache` は、未翻訳が残っても EPUB を書いた上で recoverable error にします。これは途中確認には便利ですが、完成版を上書きする危険があります。

改善案:

### 1. `recover --rebuild` は未完成なら上書きしない

`recover --rebuild` の再ビルドで cache miss が残る場合、既定では指定 output を上書きしません。

候補挙動:

```text
error: rebuild would leave 3 untranslated block(s)
partial output preserved at: .\book_jp.partial.epub
final output not overwritten: .\book_jp.epub
```

### 2. 明示オプションで部分上書きを許可する

```text
--allow-partial-output
```

これは「途中 EPUB を作る」意図を明示するためのものです。

## retry 戦略

同じ item を同じ provider/model で何度も retry して改善しない場合、サマリーで次の行動を提案します。

判定材料:

- recovery log の同一 item が繰り返し残っている。
- `failed.jsonl` に同じ source hash / reason がある。
- `unchanged_source` / `detected_untranslated_output` / `inline_restore_failed` が続く。

表示案:

```text
Strategy:
  repeated failures with deepseek-v4-flash: 17
  recommended next step: retry with a stronger model or use --manual
```

## 実装段階案

### Phase 1: 表示とドキュメントだけ

- `recover` サマリーに cache identity と execution model を表示する。
- `recover-from-cache.ps1` の説明を「別 model で不足分だけ復旧可能」と明記する。
- `Resume:` の cargo 直書き例を、可能ならスクリプト例へ寄せる。

### Phase 2: 上書き安全性

- `recover --rebuild` の再ビルドで未翻訳が残る場合、既定では final output を守る。
- `--allow-partial-output` を追加する。
- スクリプトにも `-AllowPartialOutput` を追加する。

### Phase 3: source identity

- `CacheRecord` に `source_identity` を追加する。
- 新規書き込みでは source identity を保存する。
- `cache show` に provider/model 内訳を追加する。

### Phase 4: 互換キャッシュ再利用

- `translate --reuse-compatible-cache` を追加する。
- 互換 hit 数を通常 hit と分けて表示する。
- 必要なら `--promote-compatible-cache` を追加する。

## 採用しない案

### provider / model を cache key から外す

採用しません。

理由:

- モデル差による品質差を隠してしまう。
- どの条件で生成した EPUB なのか分かりにくくなる。
- glossary や prompt version の変更と同じく、翻訳条件は cache identity に含めるべき。

### model 変更時に自動で既存訳を流用する

既定では採用しません。

理由:

- 「強い model で作り直したつもりが、古い model の訳が大量に混ざる」事故が起きる。
- 互換流用は便利だが、明示 opt-in にする。

## 結論

基本設計は次の通りです。

- 通常 `translate` は厳密 cache key で安全に中断・再開する。
- model を変えて継続したい場合は、`recover` を標準経路にする。
- `recover` は別 model で実行しても、元の recovery record の cache key に書き戻す。
- 部分出力の上書きは慎重にし、未完成 EPUB が完成版を壊さないようにする。
- provider/model をまたぐキャッシュ流用は、将来の明示 opt-in として扱う。
