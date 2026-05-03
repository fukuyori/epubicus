# OpenAI Batch 翻訳の復旧手順

この文書は、`batch run` や `batch verify` が `ERROR` で終了したあとに、何を確認し、どこから再開するかをまとめたものです。

## まず判断すること

`ERROR` 終了があっても、OpenAI Batch のリモートジョブ自体が失敗しているとは限りません。まずローカルの batch 状態を確認します。

```powershell
cargo run -- batch health .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

続いて整合性を確認します。

```powershell
cargo run -- batch verify .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

見るポイント:

- `remote parts: completed` なら、リモート Batch は完了しています。
- `missing: 0`, `stale: 0`, `orphaned: 0`, `cache_conflict: 0`, `invalid_cache: 0` なら、batch artifact とキャッシュの整合性は取れています。
- `states` に `rejected` や `failed` が残る場合、ジョブ復旧ではなく未翻訳分の補完が必要です。
- `cache_conflict` が出る場合は、同じ batch 出力を再 import してから再確認します。

## 取得済み output を再 import する

`batch verify` だけが失敗した、または `cache_conflict` が残っている場合は、リモートへ再送信せず、取得済み `output.jsonl` を再 import します。

```powershell
cargo run -- batch import .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

その後、再度 verify します。

```powershell
cargo run -- batch verify .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

## 未翻訳分だけをローカルで補完する

`rejected` や `failed` が残っている場合は、該当部分だけを `local_pending` に回します。既に `imported` / `local_imported` の項目や、有効なキャッシュがある項目は再翻訳されません。

```powershell
cargo run -- batch reroute-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --remaining `
  --priority short-first
```

ローカル provider で `local_pending` だけを翻訳します。

```powershell
cargo run -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider ollama `
  --model qwen3:14b `
  --glossary .\glossary.json
```

`batch translate-local` は、認証エラーで即停止し、課金だけ進む停滞や解決見込みの低い block は `local_exhausted` に回します。文献・参考文献のように原文維持で閉じると決めた block は `skipped` として確定できます。進捗表示、停止条件、`last_error` の読み方は [batch translate-local 運用メモ](batch-translate-local.ja.md) を参照してください。

時間がかかる場合は `--limit` を付けて分割実行します。

```powershell
cargo run -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider ollama `
  --model qwen3:14b `
  --glossary .\glossary.json `
  --limit 100
```

進捗は `batch health` で確認します。

```powershell
cargo run -- batch health .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

状態の目安:

```text
before:
  imported: 5407
  rejected: 511

after reroute-local:
  imported: 5407
  local_pending: 511

after translate-local:
  imported: 5407
  local_imported: 511
```

文献系を原文維持で確定した場合は、`skipped` が増え、`effective remaining` から外れます。

## EPUB を再生成する

補完後に verify します。

```powershell
cargo run -- batch verify .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

問題がなければ、キャッシュから EPUB を組み立て直します。

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

`partial output contains ... cache miss(es)` でエラー終了する場合は、まだ未翻訳ブロックが残っています。出力 EPUB と未翻訳レポートは作成済みです。`batch health` で残っている state を確認し、必要なら `reroute-local` と `translate-local` を繰り返します。

この状態、`recover` が `failed.jsonl` を残した状態、`scan-recovery` が未翻訳候補を検出して復旧ログを書いた状態は、復旧可能な変換失敗として終了コード `2` になります。バッチや CI では、終了コード `2` を「ログを見て復旧処理へ進める状態」、終了コード `1` を「入力や環境を直す必要がある致命的な失敗」として扱えます。

`translate` が `Recovery log:` を表示した場合は、その復旧ログから不足ブロックだけを再翻訳してキャッシュへ戻せます。復旧ログはキャッシュディレクトリ配下の `recovery\<出力EPUB名>\recovery.jsonl` に作成されます。人間向けの確認用には、同じディレクトリに `untranslated.txt` も作成されます。

```powershell
$log = ".\.batch-openai-cache\0123456789abcdef0123456789abcdef\recovery\book_jp\recovery.jsonl"
cargo run -- recover $log --list
cargo run -- recover $log `
  --provider ollama `
  --model qwen3:14b
```

全件復旧できたら、そのまま EPUB も再生成する場合は `--rebuild` を付けます。出力先を変える場合は `--output` を指定します。

```powershell
cargo run -- recover $log --provider ollama --model qwen3:14b --rebuild
cargo run -- recover --cache .\book.epub --provider ollama --model qwen3:14b --rebuild
```

一部だけ復旧したい場合は `--page` / `--block` / `--reason` / `--limit` で対象を絞ります。

```powershell
cargo run -- recover $log --page 12 --block 3
cargo run -- recover $log --reason cache_miss --limit 20
```

復旧不能な item が残った場合は、復旧ログと同じディレクトリの `failed.jsonl` を確認し、別 provider/model で再試行するか、原文維持でよいかを判断します。

自動復旧後の EPUB を外側から確認したい場合は、補助ツール `epubtr` で未翻訳候補を一覧できます。

```powershell
epubtr -u --detail .\book_jp.epub
epubtr -u --json .\book_jp.epub
```

元 EPUB と突き合わせて、epubicus の復旧ログへ戻す場合は `scan-recovery` を使います。

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
cargo run -- recover --cache .\book.epub --provider ollama --model qwen3:14b --rebuild
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b --recover --rebuild
```

手動で一部だけ差し替える場合は、元ファイルを上書きせず、修復版を別名で出力できます。

```powershell
epubtr --output .\book_jp_fixed.epub .\book_jp.epub "原文の未翻訳部分" "修正後の訳文" 1
```

## リモートに再試行したい場合

ローカル補完ではなく OpenAI Batch に再投入したい場合は、未完了分の retry request を作成します。

```powershell
cargo run -- batch retry-requests .\book.epub `
  --cache-root .\.batch-openai-cache `
  --limit 100 `
  --priority failed-first
```

このコマンドは `retry_requests.jsonl` を作成するだけで、送信はしません。既に有効なキャッシュがある項目はスキップされます。

## ロックが残っている場合

異常終了後に同じ入力 EPUB が使用中と表示される場合は、まず対象プロセスが本当に終了しているか確認します。終了済みなら通常は自動回復されます。明示的に解除する場合:

```powershell
cargo run -- unlock .\book.epub
```

まだ動作中に見える場合は解除されません。実際に動いていないことを確認できる場合だけ `--force` を使います。

```powershell
cargo run -- unlock .\book.epub --force
```
