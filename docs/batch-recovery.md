# OpenAI Batch Translation Recovery

This document explains what to check and where to resume after `batch run` or `batch verify` exits with `ERROR`.

## Decide What Failed

An `ERROR` exit does not necessarily mean the remote OpenAI Batch job failed. First inspect the local batch state.

```powershell
cargo run -- batch health .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

Then check artifact and cache consistency.

```powershell
cargo run -- batch verify .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

What to look for:

- `remote parts: completed` means the remote Batch job completed.
- `missing: 0`, `stale: 0`, `orphaned: 0`, `cache_conflict: 0`, and `invalid_cache: 0` mean the batch artifacts and translation cache are consistent.
- If `states` still includes `rejected` or `failed`, the remote job does not need recovery; the remaining untranslated items need to be filled.
- If `cache_conflict` appears, re-import the same fetched batch output and verify again.

## Re-Import Fetched Output

If only `batch verify` failed, or if `cache_conflict` remains, do not resubmit the remote job. Re-import the already fetched `output.jsonl`.

```powershell
cargo run -- batch import .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

Then verify again.

```powershell
cargo run -- batch verify .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

## Fill Only The Remaining Items Locally

If `rejected` or `failed` items remain, route only those unfinished items to `local_pending`. Items already marked `imported` / `local_imported`, or items with valid cache entries, are not translated again.

```powershell
cargo run -- batch reroute-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --remaining `
  --priority short-first
```

Translate only the `local_pending` items with a local provider.

```powershell
cargo run -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider ollama `
  --model qwen3:14b `
  --glossary .\glossary.json
```

For long runs, add `--limit` and repeat in chunks.

```powershell
cargo run -- batch translate-local .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider ollama `
  --model qwen3:14b `
  --glossary .\glossary.json `
  --limit 100
```

Check progress with `batch health`.

```powershell
cargo run -- batch health .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

Expected state flow:

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

## Rebuild The EPUB

After filling the remaining items, verify again.

```powershell
cargo run -- batch verify .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json
```

If verify is clean, rebuild the EPUB from the cache.

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

If the command prints `partial output contains ... cache miss(es)`, untranslated blocks still remain. Check the remaining states with `batch health`, then repeat `reroute-local` and `translate-local` as needed.

## Retry Remotely Instead

If you prefer to resubmit unfinished items to OpenAI Batch instead of filling them locally, create retry requests.

```powershell
cargo run -- batch retry-requests .\book.epub `
  --cache-root .\.batch-openai-cache `
  --limit 100 `
  --priority failed-first
```

This only writes `retry_requests.jsonl`; it does not submit the file. Items that already have a valid cache entry are skipped.

## Clear A Stale Lock

If a failed run leaves the input EPUB marked as in use, first check that the recorded process has actually stopped. Finished processes are usually recovered automatically. To unlock explicitly:

```powershell
cargo run -- unlock .\book.epub
```

If the process still appears active, it will not be unlocked. Use `--force` only after confirming the process is no longer running.

```powershell
cargo run -- unlock .\book.epub --force
```

