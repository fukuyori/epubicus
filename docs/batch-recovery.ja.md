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

`partial output contains ... cache miss(es)` が表示される場合は、まだ未翻訳ブロックが残っています。`batch health` で残っている state を確認し、必要なら `reroute-local` と `translate-local` を繰り返します。

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

