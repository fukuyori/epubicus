# epubicus 運用ガイド

この文書は、日常的に EPUB を翻訳するときの実行手順をまとめたものです。
細かな全オプションは README と `cargo run -- <command> --help` を確認してください。
機能横断の共通処理を追いたい場合は [共通処理メモ](common-processing.ja.md) を参照してください。

## 基本方針

- 範囲指定の `--from` / `--to` は、読書アプリ上のページ番号ではなく EPUB 内部の読む順番です。`inspect-epub.ps1` で表示される 1 始まりの spine 番号を使います。
- まず `inspect-epub.ps1` で本文ファイル番号を確認し、本文ファイル 1 個だけの試し翻訳を行ってから、全体を処理します。
- API を使う場合は、先に使用量確認か試し翻訳で費用感を確認します。
- ローカル Ollama は料金が発生しませんが、処理時間が長くなります。
- 生成結果はブロックごとにキャッシュされます。中断後は同じ入力 EPUB と同じ provider / model / glossary で再実行すると、未処理分から再開できます。
- 完了サマリーの `Resume:` には、再実行の考え方と `--partial-from-cache` で途中結果を EPUB に組み立てるコマンドが表示されます。
- 長時間の実処理は、できるだけ `scripts\*.ps1` を使います。スクリプトは内部で `cargo run --release -- ...` を実行し、cache root、同名 glossary、出力名を揃えます。
- スクリプトは原則として 3 つ以内の引数で日常実行できる形にします。特別な指示が必要な場合でも 4 つまでに収め、細かな指定は既定値、自動検出、設定ファイル、またはテンプレート側へ寄せます。

```powershell
.\scripts\inspect-epub.ps1 .\book.epub
.\scripts\usage.ps1 .\book.epub 9 9 -Provider deepseek
.\scripts\translate-deepseek.ps1 .\book.epub -From 9 -To 9
```

## 出力ファイル名

テンプレートスクリプトは、入力 EPUB と同じフォルダに `_jp` を付けた名前で出力します。

```text
D:\books\sample.epub -> D:\books\sample_jp.epub
```

## Send to Kindle と固定レイアウト

画像上に文字を重ねる EPUB、PDF 由来の EPUB、各ページに `meta name="viewport"` と絶対配置 CSS がある EPUB は、Kindle 側の変換でリフロー扱いになると画像と文章の位置がずれることがあります。

epubicus は、viewport と固定配置らしいページ構造を検出した場合、自動で OPF に Kindle 向けの `fixed-layout`、`original-resolution`、`orientation-lock` を追加します。`--kindle-fixed-layout` を付けると強制追加、`--no-kindle-fixed-layout` を付けると自動追加を無効化できます。スクリプトでは第 2 引数に `fixed` または `reflow` を指定します。

```powershell
.\scripts\rebuild-from-cache.ps1 .\book.epub
.\scripts\rebuild-from-cache.ps1 .\book.epub fixed
.\scripts\rebuild-from-cache.ps1 .\book.epub reflow
```

通常の小説や技術書のようなリフロー EPUB に固定レイアウトメタデータを付けると、Kindle 上で文字サイズ変更や画面幅に合わせた再配置が効きにくくなります。自動判定が不要な場合は、スクリプトでは `reflow`、CLI 直接実行では `--no-kindle-fixed-layout` を使ってください。

## 実行プロファイルと進捗表示

テンプレートスクリプトは、通常の長時間変換を `cargo run --release -- ...` で実行します。手動で実処理する場合も `cargo run --release -- ...` を使います。コード確認や短い dry-run だけ、デバッグビルドの `cargo run -- ...` で構いません。

ETA は前付けページを除外して測ります。spine 1〜3ページ目は計測時間と文字数に入れず、4ページ目以降で provider 翻訳が始まってから5分経つまでは `ETA pending` のままです。詳しくは [実行プロファイルと進捗表示](runtime-progress.ja.md) を参照してください。

## ローカル Ollama

PowerShell では `translate-ollama.ps1` を使います。

```powershell
.\scripts\translate-ollama.ps1 .\book.epub -Mode page -From 9 -To 9
.\scripts\translate-ollama.ps1 .\book.epub
```

キャッシュだけで EPUB を組み立てる場合:

```powershell
.\scripts\translate-ollama.ps1 .\book.epub -Mode cache
```

変数と関数だけ読み込む場合:

```powershell
. .\scripts\translate-ollama.ps1 .\book.epub -NoRun
Invoke-EpubicusLocalPageCheck -From 9 -To 9
Invoke-EpubicusLocalFull
Invoke-EpubicusAssembleFromCache
```

macOS/Linux では `.sh` 版を使います。

```sh
scripts/translate-ollama.sh ./book.epub --mode page --from 9 --to 9
scripts/translate-ollama.sh ./book.epub
```

## OpenAI / Claude / DeepSeek 通常 API

通常 API はすぐに結果を得やすい一方、未キャッシュ部分のリクエスト数に応じて課金されます。最初は使用量確認と本文ファイル 1 個だけの試し翻訳で確認してください。

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\translate-openai.ps1 .\book.epub -From 9 -To 9 -UsageOnly
.\scripts\translate-openai.ps1 .\book.epub -From 9 -To 9
```

Claude の通常 API:

```powershell
$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
.\scripts\translate-claude.ps1 .\book.epub -From 9 -To 9 -UsageOnly
.\scripts\translate-claude.ps1 .\book.epub -From 9 -To 9
```

DeepSeek の通常 API:

開始番号と終了番号は `inspect-epub.ps1` の一覧を見て選びます。次の `9 9` は、開始番号と終了番号がどちらも `9` なので、9 番目に出る本文ファイルだけを対象にします。読書アプリ上のページ番号ではありません。

```powershell
$env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput
.\scripts\usage.ps1 .\book.epub 9 9 -Provider deepseek
.\scripts\translate-deepseek.ps1 .\book.epub -From 9 -To 9
.\scripts\translate-deepseek.ps1 .\book.epub
```

`translate-deepseek.ps1` には `translate` の追加オプションをそのまま渡せます。小説向けに翻訳する場合は `--style novel`、丁寧寄りの小説文体にする場合は `--style novel-polite` を使います。

```powershell
.\scripts\translate-deepseek.ps1 .\book.epub --style novel
.\scripts\translate-deepseek.ps1 .\book.epub --style novel-polite
```

DeepSeek の model や並列数を固定で変えたい場合は `scripts\translate-deepseek.ps1` を直接編集するか、CLI オプションで上書きします。

macOS/Linux:

```sh
export OPENAI_API_KEY="..."
scripts/translate-openai.sh ./book.epub --from 9 --to 9 --usage-only

export ANTHROPIC_API_KEY="..."
scripts/translate-claude.sh ./book.epub --from 9 --to 9 --usage-only

export DEEPSEEK_API_KEY="..."
scripts/translate-deepseek.sh ./book.epub --from 9 --to 9 --usage-only
```

## OpenAI Batch API

Batch API は、分割、送信、待機、受信、取り込み、組み立てを分けて管理します。`batch run` はそれらをまとめて実行するオーケストレーションです。Claude Batch には対応しません。

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\translate-openai-batch.ps1 .\book.epub -From 9 -To 9
```

手動で状態を確認しながら進める場合:

```powershell
cargo run --release -- batch prepare .\book.epub --provider openai --model gpt-5-mini
cargo run --release -- batch submit .\book.epub --provider openai --model gpt-5-mini
cargo run --release -- batch status .\book.epub
cargo run --release -- batch fetch .\book.epub
cargo run --release -- batch import .\book.epub
cargo run --release -- batch verify .\book.epub
cargo run --release -- translate .\book.epub --partial-from-cache --keep-cache --output .\book_jp.epub
```

`batch run --wait` を使うと、完了までポーリングし、取得、取り込み、検証、指定時の EPUB 組み立てまで行います。

```powershell
cargo run --release -- batch run .\book.epub --provider openai --model gpt-5-mini --wait --poll-secs 60 --output .\book_jp.epub
```

まだ `in_progress` の場合は、同じコマンドを後で再実行できます。既存の manifest と取得済みファイルを使って再開します。

## 未翻訳が残る場合

未翻訳が残る主な原因は、未キャッシュのブロックがある状態で `--partial-from-cache` によって組み立てた場合、またはモデル出力が検証で rejected / failed になった場合です。

Batch API 実行後の復旧判断と詳細手順は [OpenAI Batch 翻訳の復旧手順](batch-recovery.ja.md) を参照してください。
`batch translate-local` の停止条件、`local_exhausted`、`skipped`、`last_error` の読み方は [batch translate-local 運用メモ](batch-translate-local.ja.md) を参照してください。

まず状態を確認します。

```powershell
cargo run --release -- batch health .\book.epub
cargo run --release -- batch verify .\book.epub
```

未完了分をローカルに回す場合:

```powershell
cargo run --release -- batch reroute-local .\book.epub --remaining --priority short-first
cargo run --release -- batch translate-local .\book.epub --provider ollama --model qwen3:14b --limit 100
cargo run --release -- batch verify .\book.epub
cargo run --release -- translate .\book.epub --partial-from-cache --keep-cache --output .\book_jp.epub
```

`translate` が `Recovery log:` を表示した場合は、復旧ログから不足ブロックだけ再翻訳できます。EPUB まで作り直す場合は `--rebuild` を付けます。

```powershell
$log = ".\.cache\<hash>\recovery\book_jp\recovery.jsonl"
cargo run --release -- recover $log --provider ollama --model qwen3:14b --rebuild
cargo run --release -- recover --cache .\book.epub --provider ollama --model qwen3:14b --rebuild
```

通常 API の cache から復旧する場合は、スクリプトを使うと cache root と同名 glossary を自動で揃えられます。DeepSeek の例:

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek
```

復旧対象を理由で絞る場合は `-Reason` を使います。主な値は次の通りです。

`recover` は失敗しやすい item を扱うため、翻訳検証に落ちた返答の再試行は `--validation-retries` / `EPUBICUS_VALIDATION_RETRIES` で制御します。既定値は 1 回です。通信失敗、rate limit、server error は `--retries` / `-r` / `EPUBICUS_RETRIES` の対象です。

| reason | 意味 | 使いどころ |
|--|--|--|
| `cache_miss` | キャッシュに訳文がない | まず通常復旧する対象 |
| `invalid_cached_translation` | 既存キャッシュ訳が現在の検証に通らない | 別 model/provider や手動訳を検討する対象 |
| `validation_passthrough` | provider 翻訳が検証に通らず原文保持された | 別 model/provider や手動訳を検討する対象 |
| `inline_restore_failed` | XHTML インライン要素の復元に失敗した | inline placeholder 対策や手動確認が必要な対象 |
| `detected_untranslated_output` | 出力済み EPUB の検査で未翻訳らしい block と判定された | `scan-recovery` 後の復旧対象 |
| `unchanged_source` | provider が原文をそのまま返した | 見出し・短文の別 model/provider、または原文保持判断 |
| `original_output` | 出力 EPUB に原文が残っている | 再翻訳するか、意図的な原文保持として扱う |

例:

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Reason cache_miss -NoRebuild
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Model deepseek-v4-pro -Reason invalid_cached_translation -Limit 20 -NoRebuild
```

出力済み EPUB を後から検査して復旧ログを作る場合:

```powershell
cargo run --release -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
cargo run --release -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b --recover --rebuild
```

出力 EPUB が `<入力名>_jp.epub` なら、スクリプトでは第 2 引数を省略できます。

```powershell
.\scripts\scan-and-recover.ps1 .\book.epub -Provider deepseek
```

PowerShell の行継続記号 `` ` `` の後ろには、空白を入れないでください。

### 手動訳を直接キャッシュへ入れる

同じブロックを同じ provider/model で再試行しても改善しない場合は、手動訳をキャッシュへ直接入れます。復旧ログの item に合わせて、`page` / `block` / `href` か `cache_key` を指定します。

```json
{
  "entries": [
    {
      "page": 23,
      "block": 2,
      "href": "text/part0021.html",
      "text": "スタジオが形を成す"
    }
  ]
}
```

```powershell
$log = ".\.cache\<hash>\recovery\book_jp\recovery.jsonl"
cargo run --release -- recover $log `
  --manual .\book.manual.json `
  --provider deepseek `
  --model deepseek-v4-flash `
  --cache-root .\.cache `
  --rebuild `
  --output .\book_jp.epub `
  --glossary .\book.json
```

この方法では、一致した item は API に送られず、指定した訳文がそのまま通常の翻訳キャッシュに保存されます。次回以降の `translate --partial-from-cache` や `recover --rebuild` でも同じキャッシュが使われます。

通常はログパスを手で指定せず、入力 EPUB から最新ログを探すスクリプトを使えます。

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Manual .\book.manual.json
```

リモート再試行用の JSONL を作る場合:

```powershell
cargo run --release -- batch retry-requests .\book.epub --limit 100 --priority failed-first
```

## キャッシュと競合

同じ入力 EPUB の同じブロックは、プロバイダ、モデル、スタイル、用語集、プロンプトバージョンなどを含むキーでキャッシュされます。

同じキーに対して別の翻訳が後から生成された場合、epubicus は既存の有効なキャッシュを優先し、後から来た差分を上書きしません。ローカルモデルの揺れや再試行によって翻訳文が少し変わっても、処理を止めずに再開しやすくするためです。

キャッシュを残しておきたい場合:

```powershell
cargo run --release -- translate .\book.epub --keep-cache --output .\book_jp.epub
```

キャッシュ管理:

```powershell
cargo run --release -- cache list
cargo run --release -- cache show .\book.epub
cargo run --release -- cache prune --older-than 30
cargo run --release -- cache clear --hash <hash>
```

`cache list` と `cache show` では、翻訳キャッシュだけでなく、同じキャッシュ配下に保存された復旧ログの件数も確認できます。`cache show` は `recover` に渡す `recovery.jsonl` のパスも表示します。`cache clear` / `cache prune` で削除すると、翻訳キャッシュ、Batch artifact、復旧ログが同じ単位で整理されます。出力済み EPUB は削除されません。

## 同時起動とロック

同一 EPUB への同時処理は入力ロックで防止されます。異常終了でロックが残った場合、記録されたプロセスが終了済みなら自動回復されます。明示的に解除する場合:

```powershell
cargo run --release -- unlock .\book.epub
```

まだ処理中に見える場合は解除されません。実際に動作していないことを確認した場合だけ `--force` を使います。

```powershell
cargo run --release -- unlock .\book.epub --force
```

## 料金確認

変換前の使用量確認:

```powershell
cargo run --release -- translate .\book.epub --provider openai --model gpt-5-mini --usage-only
```

この出力は API リクエスト数と概算トークン数です。実際の請求額は、利用するプロバイダ、モデル、Batch 割引、入力/出力単価によって変わります。大きい書籍では先に本文ファイル 1 個だけの試し翻訳で品質と費用感を確認してください。
