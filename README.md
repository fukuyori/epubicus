# epubicus

`epubicus` is a CLI tool for translating English EPUB files into Japanese while keeping the EPUB package structure and XHTML formatting intact.

It currently supports local Ollama, OpenAI, and Claude providers.

## Documentation

- [docs/README.md](docs/README.md) maps the operator guides, recovery notes, and design documents.
- [docs/batch-recovery.md](docs/batch-recovery.md) is the detailed checklist for Batch API recovery.
- [docs/operation-guide.ja.md](docs/operation-guide.ja.md) is the Japanese operator guide.

## Quick Start

Inspect the EPUB first. `FROM` and `TO` in translation commands are 1-based OPF spine numbers, not reader page numbers.

```powershell
cargo run -- inspect .\book.epub
cargo run -- toc .\book.epub
```

Translate a small range to stdout:

```powershell
cargo run -- test .\book.epub --from 1 --to 1 --provider ollama --model qwen3:14b
```

Create a translated EPUB:

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --provider ollama --model qwen3:14b
```

For long local-model generations, increase the per-request timeout and retry count:

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --provider ollama --model qwen3:14b --timeout-secs 1800 --retries 3
```

For remote providers, run several uncached requests in parallel to improve throughput:

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --provider openai --model gpt-5-mini --concurrency 4
```

To preview the estimated API request and token usage before translating, use `--usage-only`. It does not call the provider.

```powershell
cargo run -- translate .\book.epub -p openai -m gpt-5-mini -j 4 --usage-only
```

To avoid repeating common options, set `EPUBICUS_*` environment variables once in your PowerShell session:

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
$env:EPUBICUS_PROVIDER = "openai"
$env:EPUBICUS_MODEL = "gpt-5-mini"
$env:EPUBICUS_FALLBACK_PROVIDER = "ollama"
$env:EPUBICUS_FALLBACK_MODEL = "qwen3:14b"
$env:EPUBICUS_CONCURRENCY = "4"

cargo run -- translate .\book.epub -o .\book.ja.epub
```

For local Ollama testing, a PowerShell template is available:

```powershell
Copy-Item .\scripts\local-ollama-env.template.ps1 .\scripts\local-ollama-env.ps1
.\scripts\local-ollama-env.ps1 .\book.epub
```

The script sets `EPUBICUS_*` environment variables for Ollama, uses the input
EPUB as `$InputEpub`, and writes the output next to the input with `_jp`
appended to the file name:

```text
.\book.epub -> .\book_jp.epub
```

Useful modes:

```powershell
# Full local conversion
.\scripts\local-ollama-env.ps1 .\book.epub

# Page-range check
.\scripts\local-ollama-env.ps1 .\book.epub -Mode page -From 3 -To 3

# Assemble from cache without calling Ollama
.\scripts\local-ollama-env.ps1 .\book.epub -Mode cache

# Load variables and helper functions, but do not run
. .\scripts\local-ollama-env.ps1 .\book.epub -NoRun
```

For OpenAI Batch API runs, use the matching Batch template:

```powershell
Copy-Item .\scripts\openai-batch-env.template.ps1 .\scripts\openai-batch-env.ps1
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\openai-batch-env.ps1 .\book.epub
```

It also writes `.\book_jp.epub` next to the input file. Use a page range while
checking cost and quality:

```powershell
.\scripts\openai-batch-env.ps1 .\book.epub -From 3 -To 3
```

To check or resume without immediately running:

```powershell
. .\scripts\openai-batch-env.ps1 .\book.epub -NoRun
Invoke-EpubicusOpenAiBatchStatus
Invoke-EpubicusOpenAiBatchVerify
Invoke-EpubicusOpenAiBatch
```

For normal OpenAI API or Claude API runs, use the provider-specific templates:

```powershell
Copy-Item .\scripts\openai-env.template.ps1 .\scripts\openai-env.ps1
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\openai-env.ps1 .\book.epub

Copy-Item .\scripts\claude-env.template.ps1 .\scripts\claude-env.ps1
$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
.\scripts\claude-env.ps1 .\book.epub
```

Both templates support the same page-range and usage-estimate options:

```powershell
.\scripts\openai-env.ps1 .\book.epub -From 3 -To 3
.\scripts\openai-env.ps1 .\book.epub -From 3 -To 3 -UsageOnly
.\scripts\claude-env.ps1 .\book.epub -From 3 -To 3
.\scripts\claude-env.ps1 .\book.epub -From 3 -To 3 -UsageOnly
```

For macOS/Linux shells, use the `.sh` templates instead:

```bash
cp scripts/local-ollama-env.template.sh scripts/local-ollama-env.sh
chmod +x scripts/local-ollama-env.sh
scripts/local-ollama-env.sh ./book.epub

cp scripts/openai-env.template.sh scripts/openai-env.sh
chmod +x scripts/openai-env.sh
export OPENAI_API_KEY="..."
scripts/openai-env.sh ./book.epub --from 3 --to 3 --usage-only

cp scripts/claude-env.template.sh scripts/claude-env.sh
chmod +x scripts/claude-env.sh
export ANTHROPIC_API_KEY="..."
scripts/claude-env.sh ./book.epub --from 3 --to 3 --usage-only

cp scripts/openai-batch-env.template.sh scripts/openai-batch-env.sh
chmod +x scripts/openai-batch-env.sh
export OPENAI_API_KEY="..."
scripts/openai-batch-env.sh ./book.epub --from 3 --to 3
```

See [docs/operation-guide.ja.md](docs/operation-guide.ja.md) for a practical
Japanese workflow guide covering local Ollama, normal OpenAI/Claude API runs,
OpenAI Batch API runs, cache recovery, and cost checks.
Check OpenAI API usage at <https://platform.openai.com/usage> and billing at
<https://platform.openai.com/settings/organization/billing/overview>.
Multilingual input/output support is planned in
[docs/multilingual-design.md](docs/multilingual-design.md).

Translation results are cached per-input EPUB under an OS-standard cache root (Windows: `%LOCALAPPDATA%\epubicus\cache`, Unix: `~/.cache/epubicus`). Each input gets its own subdirectory named after the SHA-256 hash of its bytes, with `manifest.json` and `translations.jsonl` inside.

Provider responses are validated before they are written to the cache. Empty responses, unchanged English source text, prompt-wrapper leaks, missing inline placeholders, and likely refusal/explanation text are retried according to `--retries`. If a likely refusal/explanation still fails after retries and `--fallback-provider` is set, the original source text is translated again with the fallback provider. If the fallback also fails, the run stops without caching the bad response.

When the same cache key is produced more than once, epubicus keeps the first
valid cached translation. Identical duplicate writes are treated as already
done; different later translations for the same key are ignored instead of
overwriting the cache. This prevents nondeterministic local model output from
turning a resumable run into a hard cache conflict.

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --cache-root .\.epubicus-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --clear-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --no-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --keep-cache
```

After an interrupted run, rerun the same `translate` command to resume from uncached blocks. Because the cache directory is keyed by input EPUB hash, resuming works regardless of the output path. During parallel execution, each successful block is written to the cache immediately instead of waiting for the whole page batch to finish, so an interruption only loses blocks that were still in flight and had not returned yet. The progress bar starts at the cached position and shows a message such as `resuming: 991/5805 cached`.

Only one epubicus command may read or process the same input EPUB at a time. If a previous process was killed and left an input-use flag behind, epubicus removes it automatically when the recorded process is no longer running. You can also remove it explicitly:

```powershell
cargo run -- unlock .\book.epub
```

If the recorded process still appears to be running, `unlock` refuses to remove the flag. Use `--force` only after confirming no epubicus process is using that EPUB:

```powershell
cargo run -- unlock .\book.epub --force
```

On a successful full-range translation, the cache directory is **automatically deleted**. Pass `--keep-cache` to retain it (useful for debugging or to keep entries available for partial reuse).

Create a partial translated EPUB from cache only, leaving cache misses unchanged. This mode is **read-only on the cache** (no manifest update, no auto-delete):

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --partial-from-cache
```

To stop an in-progress translation and still finish an EPUB with the work done so far, press `Ctrl+C`, then rebuild with the same input EPUB, the same translation settings, and `--partial-from-cache`. Cached blocks are replaced with translations; missing blocks stay as original source text.

Example for an OpenAI Batch cache:

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

Example for a local Ollama cache:

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.local-ollama-cache `
  --provider ollama `
  --model qwen3:14b `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

If any block is written unchanged, epubicus writes `recovery\<output EPUB name>\untranslated.txt` under the cache directory. Use that file after noisy runs to inspect the page number, XHTML href, reason, and original source block for each untranslated output block. Because it lives with the cache, `cache clear` / `cache prune` can clean translation cache, batch artifacts, and recovery logs together.

epubicus also writes `recovery\<output EPUB name>\recovery.jsonl`. The exact path is printed at the end of `translate` as `Recovery log:`. Use the `recover` subcommand to retry only those blocks and insert successful translations back into the normal cache.

```powershell
$log = ".\.local-ollama-cache\0123456789abcdef0123456789abcdef\recovery\book_jp\recovery.jsonl"
cargo run -- recover $log
```

To recover and rebuild in one step, pass `--rebuild`. When every selected item is recovered, epubicus rebuilds the EPUB from cache with `--partial-from-cache` and the output path recorded in the recovery log.

```powershell
cargo run -- recover $log --rebuild
```

Pass `--output` to write the rebuilt EPUB elsewhere. To rebuild manually instead, run:

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.local-ollama-cache `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

If some items still cannot be recovered, epubicus writes `failed.jsonl` next to the recovery log and prints the page number, block, href, reason, last error, and cache key. To retry with another provider or model, pass `--provider` / `--model` to `recover`.

Use the same provider, model, style, and glossary as the interrupted run because they are part of the cache key.

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --provider ollama --model qwen3:14b --style tech --glossary .\glossary.json --partial-from-cache
```

If the previous run used a custom cache root, pass the same `--cache-root`:

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --cache-root .\.epubicus-cache --partial-from-cache
```

Inspect or maintain the caches:

```powershell
cargo run -- cache list
cargo run -- cache show <hash-or-input.epub>
cargo run -- cache prune --older-than 30
cargo run -- cache clear --hash <hash>
cargo run -- cache clear --all
```

Translate only a selected range and leave the rest of the EPUB unchanged:

```powershell
cargo run -- translate .\book.epub -o .\book.part-ja.epub --from 3 --to 5 --provider ollama --model qwen3:14b
```

Smoke-test the EPUB pipeline without calling any model:

```powershell
cargo run -- translate .\book.epub --from 1 --to 1 --dry-run
```

## Commands

```powershell
cargo run -- translate <INPUT.epub> [-o OUTPUT.epub] [OPTIONS]
cargo run -- test      <INPUT.epub> --from N --to M [OPTIONS]
cargo run -- inspect   <INPUT.epub>
cargo run -- toc       <INPUT.epub>
cargo run -- glossary  <INPUT.epub> [-o glossary.json]
cargo run -- cache     <SUBCOMMAND>
cargo run -- unlock    <INPUT.epub> [--force]
cargo run -- batch     <SUBCOMMAND>
```

`translate` creates an EPUB and shows a progress bar with elapsed time, ETA, selected spine pages, translatable XHTML block count, and in-flight provider request progress for uncached blocks. ETA is a simple cumulative estimate based on provider translation time per completed uncached source character, so it becomes more useful as a long run progresses. When translation moves from small startup/front-matter batches into the first substantial provider batch, the ETA baseline is reset once so long body runs are not dominated by startup pages. When the provider returns usage data, such as OpenAI or Claude, the final summary includes API request count and input / output / total tokens.

`test` prints translated text for a selected spine range to stdout. It does not create an EPUB.

`inspect` shows OPF path, spine order, `linear` state, referenced file existence, file size, and a rough count of translatable XHTML blocks.

`toc` shows EPUB3 `nav.xhtml` or EPUB2 NCX table-of-contents entries with indentation and target hrefs.

`glossary` extracts candidate proper nouns and terms into JSON for manual review.

`unlock` removes a stale input-use flag for an EPUB. Without `--force`, it only removes the flag when the recorded process is no longer running on the same host.

`batch prepare` creates local OpenAI Batch API request artifacts without making a network call. It writes compatibility `requests.jsonl` plus `requests.part-0001.jsonl` style part files; `--max-requests-per-file <N>` defaults to `50000` and `--max-bytes-per-file <N>` defaults to `200000000`. `batch run` orchestrates prepare, submit, status polling, fetch, import, health, and verify; without `--wait`, it stops after the current remote status if the batch is still running, so the same command can be re-run later. `batch submit` uploads each request part and creates one OpenAI Batch API job per part. `batch status` refreshes all remote part statuses into `batch_manifest.json`. `batch fetch` downloads missing part output/error files, reuses existing part files on rerun, and rebuilds aggregate `output.jsonl` and `remote_errors.jsonl` files. `batch import` imports the fetched `output.jsonl` into the normal translation cache, marks fetched remote error lines as `failed`, and reports `imported_with_errors` if any item failed or was rejected; already-cached identical output is reported separately and imports can be rerun. `batch retry-requests` writes `retry_requests.jsonl` for failed/rejected uncached items without submitting it. `--output <PATH>` can import another local Batch API output JSONL file. `batch reroute-local` marks selected unfinished items as `local_pending`. `batch translate-local` translates `local_pending` items through the normal provider backend and writes them to the original batch cache slots. Local fallback and retry-planning commands support `--limit <N>` and `--priority page-order|failed-first|hard-first|short-first|oldest-first` for bounded catch-up runs. `batch health` shows the local batch manifest, remote batch IDs, per-part remote status counts, work item states, request count, import report, cache-backed work, and oldest pending age. `batch verify` checks the current EPUB, `work_items.jsonl`, and cache for missing, stale, orphaned, conflicting, or invalid entries.

One-command Batch API flow:

```powershell
$env:OPENAI_API_KEY = "..."
cargo run -- batch run .\book.epub --provider openai --model gpt-5-mini --wait --poll-secs 60 --output .\book.ja.epub
```

The same command is resume-friendly. If it exits while the remote status is
still `in_progress`, run it again later; it will skip already prepared or
submitted work and continue from status/fetch/import. When `--output <PATH>` is
set, it also assembles the final EPUB from the imported cache.

Manual Batch API flow:

```powershell
$env:OPENAI_API_KEY = "..."
cargo run -- batch prepare .\book.epub --provider openai --model gpt-5-mini
cargo run -- batch submit .\book.epub --provider openai --model gpt-5-mini
cargo run -- batch status .\book.epub
cargo run -- batch fetch .\book.epub
cargo run -- batch import .\book.epub
cargo run -- translate .\book.epub --partial-from-cache --keep-cache
```

`batch verify` is useful after import or local rerouting. It compares the
current EPUB extraction, `work_items.jsonl`, and the translation cache. Missing
or invalid items can be retried remotely with `batch retry-requests` or moved
to local translation with `batch reroute-local` and `batch translate-local`.
For the full recovery checklist, see
[docs/batch-recovery.md](docs/batch-recovery.md).

If the remote batch returns failed or rejected items, either create a retry file
for later remote handling or switch the remaining work to a local provider:

```powershell
cargo run -- batch retry-requests .\book.epub --limit 100 --priority failed-first
cargo run -- batch reroute-local .\book.epub --remaining --priority short-first
cargo run -- batch translate-local .\book.epub --provider ollama --model qwen3:14b --limit 100
```

## Options

### `translate`

| Option | Default | Description |
|--|--|--|
| `-o, --output PATH` | `<input>.ja.epub` | Output EPUB |
| `--from N` | first spine item | First 1-based OPF spine number to translate |
| `--to N` | last spine item | Last 1-based OPF spine number to translate |
| `--partial-from-cache` | false | Replace cache hits with translations and keep cache misses unchanged. If untranslated blocks remain, write the EPUB and report, then exit with an error |

When an EPUB and recovery log were written but untranslated blocks remain, `recover` leaves unrecoverable items in `failed.jsonl`, or `scan-recovery` detects suspicious untranslated blocks and writes a recovery log, epubicus exits with code `2` for a recoverable error. Non-recoverable failures such as invalid input EPUBs or unwritable output paths use the normal error code `1`.

### `test`

| Option | Default | Description |
|--|--|--|
| `--from N` | required | First 1-based OPF spine number to print |
| `--to N` | required | Last 1-based OPF spine number to print |

### Shared `translate` / `test` Options

CLI arguments take precedence over environment variables.

| Option | Environment variable | Default | Description |
|--|--|--|--|
| `-p, --provider ollama\|openai\|claude` | `EPUBICUS_PROVIDER` | `ollama` | Translation provider |
| `-m, --model NAME` | `EPUBICUS_MODEL` | provider-specific | Model name |
| `--fallback-provider ollama\|openai\|claude` | `EPUBICUS_FALLBACK_PROVIDER` | none | Fallback provider used only when the primary provider returns a likely refusal/explanation and retries are exhausted |
| `--fallback-model NAME` | `EPUBICUS_FALLBACK_MODEL` | fallback-provider-specific | Model name for the fallback provider |
| `--ollama-host URL` | `EPUBICUS_OLLAMA_HOST` | `http://localhost:11434` | Ollama endpoint |
| `--openai-base-url URL` | `EPUBICUS_OPENAI_BASE_URL` | `https://api.openai.com/v1` | OpenAI API base URL |
| `--claude-base-url URL` | `EPUBICUS_CLAUDE_BASE_URL` | `https://api.anthropic.com/v1` | Claude / Anthropic API base URL |
| `--openai-api-key KEY` | `OPENAI_API_KEY` | none | OpenAI API key. `--openai-api-key` takes precedence |
| `--anthropic-api-key KEY` | `ANTHROPIC_API_KEY` | none | Anthropic API key. `--anthropic-api-key` takes precedence |
| `--prompt-api-key` | none | false | Prompt for the provider API key without echoing it |
| `-T, --temperature F` | `EPUBICUS_TEMPERATURE` | `0.3` | Sampling temperature |
| `-n, --num-ctx N` | `EPUBICUS_NUM_CTX` | `8192` | Context window size passed to Ollama |
| `-t, --timeout-secs N` | `EPUBICUS_TIMEOUT_SECS` | `900` | HTTP timeout per request, in seconds |
| `-r, --retries N` | `EPUBICUS_RETRIES` | `3` | Retries after the initial attempt for timeouts, connection errors, rate limits, server errors, and validation failures |
| `-x, --max-chars-per-request N` | `EPUBICUS_MAX_CHARS_PER_REQUEST` | `3500` | Split longer XHTML text blocks into multiple provider requests at sentence boundaries. Use `0` to disable splitting |
| `-j, --concurrency N` | `EPUBICUS_CONCURRENCY` | `1` | Run up to N uncached provider requests in parallel per XHTML file. The effective concurrency is automatically reduced after retryable errors such as rate limits, timeouts, and server errors, then slowly restored after successful requests |
| `-s, --style STYLE` | `EPUBICUS_STYLE` | `essay` | Style preset: `novel`, `novel-polite`, `tech`, `essay`, `academic`, `business` |
| `-d, --dry-run` | none | false | Do not call a provider; use source text to smoke-test EPUB handling |
| `-g, --glossary PATH` | none | none | Glossary JSON for consistent terms |
| `--cache-root PATH` | none | OS cache (`%LOCALAPPDATA%\epubicus\cache` / `~/.cache/epubicus`) | Override the cache root. Per-EPUB caches live under `<cache-root>/<input-hash>/` |
| `--no-cache` | none | false | Do not read or write the cache. Existing cache files are not deleted |
| `--clear-cache` | none | false | Delete this input EPUB's cache before translating |
| `-k, --keep-cache` | none | false | Keep the cache after a successful completion (default: cache is auto-deleted) |
| `-u, --usage-only` | none | false | Do not call a provider; only print estimated API requests and tokens for the selected pages |
| `--passthrough-on-validation-failure` | `EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE` | false | Keep the original block in the current output after validation retries are exhausted. It is not cached, so it can be retried later. Useful for TOC/index entries where preserving links and inline structure is safer than aborting |
| `--verbose` | `EPUBICUS_VERBOSE` | false | Show detailed processing warnings such as retries, concurrency changes, fallback use, and long-block splitting |

### `recover`

| Option | Default | Description |
|--|--|--|
| `LOG` | required unless `--cache` is used | `recovery.jsonl` printed by `translate` as `Recovery log:` |
| `--cache TARGET` | none | Resolve the newest `recovery.jsonl` from an input EPUB path or cache hash prefix |
| `--input PATH` | `input_epub` from the recovery log | Explicit input EPUB |
| `--limit N` | all items | Maximum number of items to retry |
| `--list` | false | List matching recovery log items without translating |
| `--page N` | all pages | Only include records for this spine page |
| `--block N` | all blocks | Only include records for this block index |
| `--reason REASON` | all reasons | Only include records with this reason. Can be repeated |
| `--failed-log PATH` | `<LOG directory>\failed.jsonl` | Output path for unrecoverable items |
| `--rebuild` | false | Rebuild the EPUB from cache when every selected item is recovered |
| `--output PATH` | `output_epub` from the recovery log | Output EPUB path for `--rebuild` |

Examples:

```powershell
cargo run -- recover $log --list
cargo run -- recover $log --page 12 --block 3
cargo run -- recover $log --reason cache_miss --limit 20
cargo run -- recover $log --rebuild
cargo run -- recover --cache .\book.epub --rebuild
```

### `scan-recovery`

Compare a finished or partial EPUB with the original input EPUB and write `recovery.jsonl` for blocks that still look untranslated. The files are written under the input EPUB cache, using the same `recovery\<output EPUB name>\` layout as normal partial output recovery logs.

| Option | Default | Description |
|--|--|--|
| `INPUT` | required | Original input EPUB |
| `OUTPUT` | required | Translated or partially translated EPUB to inspect |
| `--limit N` | all items | Maximum number of suspicious blocks to record |
| `--recover` | false | Retry detected blocks immediately after writing the recovery log |
| `--rebuild` | false | Rebuild the inspected EPUB after `--recover` succeeds |
| `--failed-log PATH` | `<recovery log directory>\failed.jsonl` | Output path for unrecoverable items during `--recover` |

Examples:

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
cargo run -- recover --cache .\book.epub --rebuild
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b --recover --rebuild
```

Provider-specific `--model` defaults:

| Provider | Default model |
|--|--|
| `ollama` | `qwen3:14b` |
| `openai` | `gpt-5-mini` |
| `claude` | `claude-sonnet-4-5` |

### `glossary`

| Option | Default | Description |
|--|--|--|
| `-o, --output PATH` | `glossary.json` | Output glossary candidate JSON |
| `--min-occurrences N` | `3` | Minimum occurrence count for a candidate |
| `--max-entries N` | `200` | Maximum number of candidates to output |
| `--review-prompt PATH` | none | Write a Markdown prompt for reviewing the glossary with ChatGPT or Claude |

### `inspect` / `toc`

`inspect` and `toc` only take `INPUT.epub`; they have no additional options.

### `cache`

| Subcommand | Description |
|--|--|
| `cache list` | List all cached runs with hash, segment count, recovery log count, size, last update, and input path |
| `cache show <hash\|input.epub>` | Print the manifest plus recovery log locations and counts, including the `recovery.jsonl` path to pass to `recover` |
| `cache prune --older-than <DAYS> [--yes] [--dry-run]` | Delete runs whose `last_updated_at` is older than N days |
| `cache clear --hash <HASH> [--dry-run]` | Delete one cached run |
| `cache clear --all [--yes] [--dry-run]` | Delete every cached run. Requires typing `yes` unless `--yes` is set |

`cache` accepts `--cache-root <PATH>` to operate on a non-default cache root.

## Providers

Ollama is the default provider and runs locally:

The asynchronous OpenAI Batch API workflow is designed in
[docs/batch-api-design.md](docs/batch-api-design.md), with the implementation
plan in
[docs/batch-api-implementation-plan.md](docs/batch-api-implementation-plan.md).
The current implementation supports the local `batch prepare`,
`batch run`, `batch retry-requests`, `batch import`, `batch health`,
`batch verify`, and OpenAI `batch submit/status/fetch` stages, including
request-count and byte-count based multi-part Batch API jobs.

```powershell
cargo run -- test .\book.epub --from 1 --to 1 --provider ollama --model qwen3:14b
```

If `ollama` is not on PATH, run Ollama with the full path separately:

```powershell
& 'C:\Users\n_fuk\AppData\Local\Programs\Ollama\ollama.exe' pull qwen3:14b
& 'C:\Users\n_fuk\AppData\Local\Programs\Ollama\ollama.exe' list
```

OpenAI uses the Responses API. Set `OPENAI_API_KEY`, pass `--openai-api-key`, or use `--prompt-api-key`:

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
cargo run -- test .\book.epub --from 1 --to 1 --provider openai --model gpt-5-mini
```

Claude uses the Anthropic Messages API. Set `ANTHROPIC_API_KEY`, pass `--anthropic-api-key`, or use `--prompt-api-key`:

```powershell
$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
cargo run -- test .\book.epub --from 1 --to 1 --provider claude --model claude-sonnet-4-5
```

Interactive key prompt:

```powershell
cargo run -- test .\book.epub --from 1 --to 1 --provider openai --prompt-api-key
cargo run -- test .\book.epub --from 1 --to 1 --provider claude --prompt-api-key
```

## Glossary

Generate candidates:

```powershell
cargo run -- glossary .\book.epub -o .\glossary.json
```

You can also write a prompt for reviewing the candidates with ChatGPT or Claude:

```powershell
cargo run -- glossary .\book.epub -o .\glossary.candidates.json --review-prompt .\glossary-review.md
```

Send `glossary-review.md` to ChatGPT or Claude, then save the returned JSON as `glossary.json` and use it for translation. The prompt asks the model to remove false positives, merge duplicates, and fill Japanese `dst` suggestions.

`glossary-review.md` is self-contained: it includes explanatory comments, field meanings, review rules, and the candidate JSON, so it can be pasted directly into ChatGPT or Claude. `glossary.candidates.json` remains valid comment-free JSON.

Edit `dst` values:

```json
{
  "source_lang": "en",
  "target_lang": "ja",
  "entries": [
    {
      "src": "Horizon",
      "dst": "ホライゾン"
    }
  ]
}
```

Use the glossary during translation:

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --glossary .\glossary.json
```

Only entries whose `src` appears in the current block are sent to the provider, so the prompt does not include the entire glossary every time.
During translation, the provider only receives `src => dst`. Existing glossary files may still contain `kind` and `note`, but they are not included in the translation prompt. Leading/trailing whitespace in `src` / `dst` is ignored, and entries with an empty `dst` are not used during translation.

## Current Scope

- EPUB unpack and repack.
- OPF container, manifest, and spine parsing.
- OPF spine status inspection.
- EPUB3 nav / EPUB2 NCX table-of-contents display.
- Glossary candidate extraction and glossary-guided translation.
- Per-input-EPUB translation cache (keyed by SHA-256 hash) with auto-deletion on successful completion and a `cache` subcommand for list/show/prune/clear.
- Partial EPUB output from cached blocks only (read-only on the cache).
- XHTML block traversal for `p`, headings, list items, table cells, captions, footnote `aside`, and related block tags.
- Inline tag placeholder preservation with `⟦E1⟧`, `⟦/E1⟧`, and `⟦S1⟧`.
- Inline link preservation for footnote links and body links.
- Ollama `/api/chat`, OpenAI `/responses`, and Claude `/messages` translation.
- Style presets.
- Production EPUB output mode.
- Progress bar for production translation.
- Test stdout mode for selected spine pages.
- Output validation before cache writes, including retry for likely
  untranslated English, refusal/explanation text, and missing inline
  placeholders.
- OpenAI Batch API prepare/submit/status/fetch/import/verify/run workflow with
  multi-part request files and local fallback routing.
- Environment template scripts for PowerShell and POSIX shells.

## Limitations

- EPUB reader page numbers are not used. Ranges are OPF spine numbers.
- `--partial-from-cache` does not call a model, replaces cache hits with translated text, and leaves cache misses unchanged. If untranslated blocks remain, the command exits with an error after writing the EPUB and untranslated report. It cannot be combined with `--no-cache`.
- `nav.xhtml` / NCX display is implemented, but TOC translation is not implemented yet.
- Detailed fallback reports are not implemented yet.
- Code/preformatted content is protected from translation.
- `--usage-only` estimates request and token volume before the provider is
  called, but it does not calculate provider-specific currency cost.

## Troubleshooting

If `failed to open .\book.epub` appears, the file does not exist at that path. `book.epub` is only an example name.

```powershell
Get-ChildItem -Filter *.epub
cargo run -- inspect .\actual-file-name.epub
```

If `ollama` is not found, either add Ollama to PATH or use the full executable path shown above.
