# EPUB 翻訳手順

この文書は、1冊の EPUB を翻訳するときの実作業の流れをまとめたものです。
細かなオプション一覧は [README.ja.md](../README.ja.md) を参照してください。

## 全体の流れ

1. EPUB の場所を決める
2. 同じディレクトリに glossary 候補を作る
3. glossary の `dst` をレビューして訳語を入れる
4. 変換方式を選んで翻訳する
5. 未翻訳やエラーが残ったら、方式に応じてリカバリーする
6. 完成した EPUB とキャッシュ状態を確認する

スクリプトは、入力 EPUB と同じディレクトリに `_jp` 付きの EPUB を作ります。

```text
D:\books\sample.epub -> D:\books\sample_jp.epub
```

glossary も同じディレクトリ、同じベース名に揃えます。

```text
D:\books\sample.epub -> D:\books\sample.json
D:\books\sample.epub -> D:\books\sample.md
```

翻訳用スクリプトは、同名 JSON が存在する場合、自動で `--glossary` として使います。明示的に `--glossary` / `-g` を渡した場合は、明示指定が優先されます。

## glossary の作り方

まず候補 JSON とレビュー用 Markdown を作ります。

```powershell
.\scripts\create-glossary.ps1 .\book.epub
```

既に `book.json` や `book.md` がある場合は上書きしません。作り直す場合だけ `-Force` を付けます。

```powershell
.\scripts\create-glossary.ps1 .\book.epub -Force
```

実行内容だけ確認する場合:

```powershell
.\scripts\create-glossary.ps1 .\book.epub -NoRun
```

生成されるファイル:

```text
book.json  glossary 候補。翻訳時に使うファイル。
book.md    ChatGPT / Claude などで候補をレビューするためのプロンプト。
```

`book.md` を AI に渡し、誤検出の削除、重複統合、`dst` の訳語入力を行います。翻訳時に使われるのは `src => dst` です。`dst` が空の entry は翻訳時には使われません。

最小形は次の通りです。

```json
{
  "source_lang": "en",
  "target_lang": "ja",
  "entries": [
    {
      "src": "Cognitive Warfare",
      "dst": "認知戦"
    }
  ]
}
```

## 事前確認

EPUB の spine 番号を確認します。`--from` / `--to` はリーダー上のページ番号ではなく、OPF spine の 1 始まり番号です。

```powershell
cargo run -- inspect .\book.epub
cargo run -- toc .\book.epub
```

小さい範囲で処理できるか確認してから全体変換に進みます。

```powershell
cargo run -- translate .\book.epub --from 3 --to 3 --dry-run
```

以降のスクリプトは本番向けに `cargo run --release -- ...` を使うものがあります。release バイナリ更新中に実行中プロセスへ影響する場合があるため、開発中の確認だけなら `cargo build` と `target\debug\epubicus.exe` で確認してください。

## 方式1: ローカル Ollama

料金をかけずに進めたい場合の基本方式です。時間はかかりますが、中断してもキャッシュから再開できます。

初回だけテンプレートをコピーします。

```powershell
Copy-Item .\scripts\local-ollama-env.template.ps1 .\scripts\local-ollama-env.ps1
```

1ページだけ確認します。

```powershell
.\scripts\local-ollama-env.ps1 .\book.epub -Mode page -From 3 -To 3
```

全体を変換します。

```powershell
.\scripts\local-ollama-env.ps1 .\book.epub
```

キャッシュ済み分だけで EPUB を組み立てる場合:

```powershell
.\scripts\local-ollama-env.ps1 .\book.epub -Mode cache
```

### Ollama のリカバリー

途中で止まった場合は、同じコマンドを再実行します。成功済みブロックはキャッシュから飛ばされ、未処理分から続きます。

未翻訳が残った EPUB ができた場合は、復旧ログを使います。`translate` の最後に `Recovery log:` と表示されたパスを指定します。

```powershell
$log = ".\.local-ollama-cache\<hash>\recovery\book_jp\recovery.jsonl"
cargo run -- recover $log --provider ollama --model qwen3:14b --rebuild
```

ログの場所が分からない場合は、入力 EPUB から最新ログを探せます。

```powershell
cargo run -- recover --cache .\book.epub --provider ollama --model qwen3:14b --rebuild
```

同じことはスクリプトでも実行できます。`-NoRun` を付けると、実行せずコマンドだけ確認します。

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub `
  -CacheRoot .\.local-ollama-cache `
  -Provider ollama `
  -Model qwen3:14b `
  -NoRun

.\scripts\recover-from-cache.ps1 .\book.epub `
  -CacheRoot .\.local-ollama-cache `
  -Provider ollama `
  -Model qwen3:14b
```

出力済み EPUB に未翻訳が混ざっていないか後から確認する場合:

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b --recover --rebuild
```

後検査と復旧をまとめて行うスクリプト:

```powershell
.\scripts\scan-and-recover.ps1 .\book.epub .\book_jp.epub `
  -CacheRoot .\.local-ollama-cache `
  -Provider ollama `
  -Model qwen3:14b
```

## 方式2: OpenAI / Claude 通常 API

小さめの本、またはすぐに結果を見たい場合に使います。未キャッシュ部分のリクエスト数に応じて課金されます。

OpenAI:

```powershell
Copy-Item .\scripts\openai-env.template.ps1 .\scripts\openai-env.ps1
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
```

まず使用量だけ確認します。

```powershell
.\scripts\openai-env.ps1 .\book.epub -From 3 -To 3 -UsageOnly
```

小範囲で品質を確認します。

```powershell
.\scripts\openai-env.ps1 .\book.epub -From 3 -To 3
```

全体を変換します。

```powershell
.\scripts\openai-env.ps1 .\book.epub
```

Claude:

```powershell
Copy-Item .\scripts\claude-env.template.ps1 .\scripts\claude-env.ps1
$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
.\scripts\claude-env.ps1 .\book.epub -From 3 -To 3 -UsageOnly
.\scripts\claude-env.ps1 .\book.epub -From 3 -To 3
.\scripts\claude-env.ps1 .\book.epub
```

### 通常 API のリカバリー

途中で止まった場合は同じスクリプトを再実行します。キャッシュ済みブロックは再利用されます。

未翻訳レポートや復旧ログが出た場合は、`recover` で不足分だけ再翻訳します。OpenAI で失敗した箇所を Ollama に回すなど、provider を変えても構いません。

```powershell
cargo run -- recover --cache .\book.epub --provider ollama --model qwen3:14b --rebuild
```

OpenAI 用キャッシュから復旧する例:

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub `
  -CacheRoot .\.openai-cache `
  -Provider ollama `
  -Model qwen3:14b
```

Claude 用キャッシュから復旧する例:

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub `
  -CacheRoot .\.claude-cache `
  -Provider ollama `
  -Model qwen3:14b
```

完成後に不安がある場合は `scan-recovery` で検査します。

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
```

検出した未翻訳候補をそのまま復旧して EPUB を作り直す場合:

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b --recover --rebuild
```

スクリプトで行う場合:

```powershell
.\scripts\scan-and-recover.ps1 .\book.epub .\book_jp.epub `
  -CacheRoot .\.openai-cache `
  -Provider ollama `
  -Model qwen3:14b
```

## 方式3: OpenAI Batch API

大きい本をまとめて処理する場合の方式です。送信後はリモート側で非同期処理されます。進捗は `batch status` / `batch health` で確認します。

初回だけテンプレートをコピーします。

```powershell
Copy-Item .\scripts\openai-batch-env.template.ps1 .\scripts\openai-batch-env.ps1
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
```

小範囲で確認します。

```powershell
.\scripts\openai-batch-env.ps1 .\book.epub -From 3 -To 3
```

全体を実行します。

```powershell
.\scripts\openai-batch-env.ps1 .\book.epub
```

このスクリプトは `.batch-openai-cache` を使います。通常の OS 標準キャッシュとは別の場所なので、キャッシュ掃除時は `--cache-root` を合わせるか、全キャッシュ掃除用スクリプトを使います。

状態確認だけ行う場合:

```powershell
. .\scripts\openai-batch-env.ps1 .\book.epub -NoRun
Invoke-EpubicusOpenAiBatchStatus
Invoke-EpubicusOpenAiBatchVerify
```

手動で段階実行する場合:

```powershell
cargo run -- batch prepare .\book.epub --provider openai --model gpt-5-mini --cache-root .\.batch-openai-cache
cargo run -- batch submit .\book.epub --provider openai --model gpt-5-mini --cache-root .\.batch-openai-cache
cargo run -- batch status .\book.epub --cache-root .\.batch-openai-cache
cargo run -- batch fetch .\book.epub --cache-root .\.batch-openai-cache
cargo run -- batch import .\book.epub --cache-root .\.batch-openai-cache
cargo run -- batch verify .\book.epub --cache-root .\.batch-openai-cache
```

取り込み済みキャッシュから EPUB を組み立てます。

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

同名 glossary がある場合、テンプレートスクリプトは自動で使います。手動コマンドでは必要に応じて `--glossary .\book.json` を付けます。

### Batch のリカバリー

まず状態を確認します。

```powershell
cargo run -- batch health .\book.epub --cache-root .\.batch-openai-cache
cargo run -- batch verify .\book.epub --cache-root .\.batch-openai-cache
```

一連の流れは `batch-recover-local.ps1` にまとめています。まず `-NoRun` で内容を確認します。

```powershell
.\scripts\batch-recover-local.ps1 .\book.epub -NoRun
```

実行すると、次の順に処理します。

1. `batch health`
2. `batch fetch`
3. `batch import`
4. `batch reroute-local --remaining`
5. `batch translate-local`
6. `batch verify`
7. `batch health`
8. `translate --partial-from-cache` で EPUB 再組み立て

```powershell
.\scripts\batch-recover-local.ps1 .\book.epub `
  -LocalProvider ollama `
  -LocalModel qwen3:14b `
  -Limit 100
```

`batch verify` が artifact inconsistency を検出しても、リカバリー中は EPUB 再組み立てまで進めたい場合があります。そのため `batch-recover-local.ps1` は既定では verify 失敗を警告にして続行します。verify 失敗で止めたい場合だけ `-StrictVerify` を付けます。

```powershell
.\scripts\batch-recover-local.ps1 .\book.epub -StrictVerify
```

OpenAI 側の処理が完了しているのに取り込みだけ済んでいない場合:

```powershell
cargo run -- batch fetch .\book.epub --cache-root .\.batch-openai-cache
cargo run -- batch import .\book.epub --cache-root .\.batch-openai-cache
cargo run -- batch verify .\book.epub --cache-root .\.batch-openai-cache
```

failed / rejected / local_exhausted などが残る場合、少数ならローカルLLMへ回します。

```powershell
cargo run -- batch reroute-local .\book.epub --cache-root .\.batch-openai-cache --remaining --priority short-first
cargo run -- batch translate-local .\book.epub --cache-root .\.batch-openai-cache --provider ollama --model qwen3:14b --limit 100
cargo run -- batch verify .\book.epub --cache-root .\.batch-openai-cache
```

fetch/import 済みで、ローカル処理だけ再実行したい場合:

```powershell
.\scripts\batch-recover-local.ps1 .\book.epub -SkipFetchImport
```

EPUB の再組み立てを後回しにしたい場合:

```powershell
.\scripts\batch-recover-local.ps1 .\book.epub -SkipRebuild
```

OpenAI Batch で再処理したい場合は、リトライ用 JSONL を作ります。

```powershell
cargo run -- batch retry-requests .\book.epub --cache-root .\.batch-openai-cache --limit 100 --priority failed-first
```

最終的に `effective remaining: 0` になったら、EPUB を組み立て直します。

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

## 完了確認

変換方式に関係なく、最後は次を確認します。

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
```

または:

```powershell
.\scripts\scan-and-recover.ps1 .\book.epub .\book_jp.epub -ScanOnly
```

未翻訳候補が出なければ完了です。候補が出た場合は `recover` または `scan-recovery --recover --rebuild` で不足分を埋めます。

Batch の場合は、あわせて health を確認します。

```powershell
cargo run -- batch health .\book.epub --cache-root .\.batch-openai-cache
```

目安:

```text
cache-backed: 全件/全件
effective remaining: 0
```

## キャッシュ掃除

通常キャッシュだけなら `cache clear` を使います。

```powershell
cargo run -- cache clear --all
```

スクリプトは方式ごとに `.openai-cache`、`.batch-openai-cache`、`.local-ollama-cache`、`.claude-cache` を使うため、通常キャッシュ掃除だけでは残る場合があります。まとめて確認するには:

```powershell
.\scripts\clear-all-caches.ps1 -DryRun
```

実際に消す場合だけ:

```powershell
.\scripts\clear-all-caches.ps1 -Yes
```

このスクリプトは実行中の `epubicus` プロセスを止めません。ロックされて削除できないファイルは警告として残ります。
