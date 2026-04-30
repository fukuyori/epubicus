# epubicus 運用ガイド

この文書は、日常的に EPUB を翻訳するときの実行手順をまとめたものです。
細かな全オプションは README と `cargo run -- <command> --help` を確認してください。

## 基本方針

- 範囲指定の `--from` / `--to` は、読書アプリ上のページ番号ではなく OPF spine の 1 始まり番号です。
- まず `inspect` と小さい範囲で確認してから、全体を処理します。
- API を使う場合は、先に `--usage-only` か小範囲で費用感を確認します。
- ローカル Ollama は料金が発生しませんが、処理時間が長くなります。
- 生成結果はキャッシュされます。中断後は同じ入力 EPUB と同じ設定で再実行すると、未処理分から再開できます。

```powershell
cargo run -- inspect .\book.epub
cargo run -- toc .\book.epub
cargo run -- translate .\book.epub --from 3 --to 3 --dry-run
```

## 出力ファイル名

テンプレートスクリプトは、入力 EPUB と同じフォルダに `_jp` を付けた名前で出力します。

```text
D:\books\sample.epub -> D:\books\sample_jp.epub
```

## ローカル Ollama

PowerShell ではテンプレートをコピーして使います。コピーしたファイルは `.gitignore` 対象なので、モデル名や並列数を自分用に変更できます。

```powershell
Copy-Item .\scripts\local-ollama-env.template.ps1 .\scripts\local-ollama-env.ps1
.\scripts\local-ollama-env.ps1 .\book.epub -Mode page -From 3 -To 3
.\scripts\local-ollama-env.ps1 .\book.epub
```

キャッシュだけで EPUB を組み立てる場合:

```powershell
.\scripts\local-ollama-env.ps1 .\book.epub -Mode cache
```

変数と関数だけ読み込む場合:

```powershell
. .\scripts\local-ollama-env.ps1 .\book.epub -NoRun
Invoke-EpubicusLocalPageCheck -From 3 -To 3
Invoke-EpubicusLocalFull
Invoke-EpubicusAssembleFromCache
```

macOS/Linux では `.sh` テンプレートを使います。

```sh
cp scripts/local-ollama-env.template.sh scripts/local-ollama-env.sh
chmod +x scripts/local-ollama-env.sh
scripts/local-ollama-env.sh ./book.epub --mode page --from 3 --to 3
scripts/local-ollama-env.sh ./book.epub
```

## OpenAI / Claude 通常 API

通常 API はすぐに結果を得やすい一方、未キャッシュ部分のリクエスト数に応じて課金されます。最初は `--usage-only` と小範囲で確認してください。

```powershell
Copy-Item .\scripts\openai-env.template.ps1 .\scripts\openai-env.ps1
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\openai-env.ps1 .\book.epub -From 3 -To 3 -UsageOnly
.\scripts\openai-env.ps1 .\book.epub -From 3 -To 3
```

Claude の通常 API:

```powershell
Copy-Item .\scripts\claude-env.template.ps1 .\scripts\claude-env.ps1
$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
.\scripts\claude-env.ps1 .\book.epub -From 3 -To 3 -UsageOnly
.\scripts\claude-env.ps1 .\book.epub -From 3 -To 3
```

macOS/Linux:

```sh
cp scripts/openai-env.template.sh scripts/openai-env.sh
chmod +x scripts/openai-env.sh
export OPENAI_API_KEY="..."
scripts/openai-env.sh ./book.epub --from 3 --to 3 --usage-only

cp scripts/claude-env.template.sh scripts/claude-env.sh
chmod +x scripts/claude-env.sh
export ANTHROPIC_API_KEY="..."
scripts/claude-env.sh ./book.epub --from 3 --to 3 --usage-only
```

## OpenAI Batch API

Batch API は、分割、送信、待機、受信、取り込み、組み立てを分けて管理します。`batch run` はそれらをまとめて実行するオーケストレーションです。Claude Batch には対応しません。

```powershell
Copy-Item .\scripts\openai-batch-env.template.ps1 .\scripts\openai-batch-env.ps1
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\openai-batch-env.ps1 .\book.epub -From 3 -To 3
```

手動で状態を確認しながら進める場合:

```powershell
cargo run -- batch prepare .\book.epub --provider openai --model gpt-5-mini
cargo run -- batch submit .\book.epub --provider openai --model gpt-5-mini
cargo run -- batch status .\book.epub
cargo run -- batch fetch .\book.epub
cargo run -- batch import .\book.epub
cargo run -- batch verify .\book.epub
cargo run -- translate .\book.epub --partial-from-cache --keep-cache --output .\book_jp.epub
```

`batch run --wait` を使うと、完了までポーリングし、取得、取り込み、検証、指定時の EPUB 組み立てまで行います。

```powershell
cargo run -- batch run .\book.epub --provider openai --model gpt-5-mini --wait --poll-secs 60 --output .\book_jp.epub
```

まだ `in_progress` の場合は、同じコマンドを後で再実行できます。既存の manifest と取得済みファイルを使って再開します。

## 未翻訳が残る場合

未翻訳が残る主な原因は、未キャッシュのブロックがある状態で `--partial-from-cache` によって組み立てた場合、またはモデル出力が検証で rejected / failed になった場合です。

まず状態を確認します。

```powershell
cargo run -- batch health .\book.epub
cargo run -- batch verify .\book.epub
```

未完了分をローカルに回す場合:

```powershell
cargo run -- batch reroute-local .\book.epub --remaining --priority short-first
cargo run -- batch translate-local .\book.epub --provider ollama --model qwen3:14b --limit 100
cargo run -- batch verify .\book.epub
cargo run -- translate .\book.epub --partial-from-cache --keep-cache --output .\book_jp.epub
```

リモート再試行用の JSONL を作る場合:

```powershell
cargo run -- batch retry-requests .\book.epub --limit 100 --priority failed-first
```

## キャッシュと競合

同じ入力 EPUB の同じブロックは、プロバイダ、モデル、スタイル、用語集、プロンプトバージョンなどを含むキーでキャッシュされます。

同じキーに対して別の翻訳が後から生成された場合、epubicus は既存の有効なキャッシュを優先し、後から来た差分を上書きしません。ローカルモデルの揺れや再試行によって翻訳文が少し変わっても、処理を止めずに再開しやすくするためです。

キャッシュを残しておきたい場合:

```powershell
cargo run -- translate .\book.epub --keep-cache --output .\book_jp.epub
```

キャッシュ管理:

```powershell
cargo run -- cache list
cargo run -- cache show .\book.epub
cargo run -- cache prune --older-than 30
cargo run -- cache clear --hash <hash>
```

## 同時起動とロック

同一 EPUB への同時処理は入力ロックで防止されます。異常終了でロックが残った場合、記録されたプロセスが終了済みなら自動回復されます。明示的に解除する場合:

```powershell
cargo run -- unlock .\book.epub
```

まだ処理中に見える場合は解除されません。実際に動作していないことを確認した場合だけ `--force` を使います。

```powershell
cargo run -- unlock .\book.epub --force
```

## 料金確認

変換前の使用量確認:

```powershell
cargo run -- translate .\book.epub --provider openai --model gpt-5-mini --usage-only
```

この出力は API リクエスト数と概算トークン数です。実際の請求額は、利用するプロバイダ、モデル、Batch 割引、入力/出力単価によって変わります。大きい書籍では先に小範囲で品質と費用感を確認してください。
