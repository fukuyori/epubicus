# epubicus

`epubicus` is a CLI tool for translating EPUB files from multiple source languages into Japanese while keeping the EPUB package structure and XHTML formatting intact.

It currently supports local Ollama, OpenAI, Claude, and DeepSeek providers.

## Documentation

- [docs/README.md](docs/README.md) maps the operator guides, recovery notes, and design documents.
- [docs/scripts-reference.ja.md](docs/scripts-reference.ja.md) is the per-script reference (purpose, arguments, usage examples) for `scripts/`.
- [README.ja.md](README.ja.md) is the primary Japanese quick start and script-first command reference.
- [docs/translation-workflow.ja.md](docs/translation-workflow.ja.md) is a Japanese step-by-step workflow for glossary creation, translation methods, and recovery.
- [docs/operation-guide.ja.md](docs/operation-guide.ja.md) is the Japanese operator guide.
- [docs/detailed-examples.ja.md](docs/detailed-examples.ja.md) has detailed command examples, cache operations, and Send to Kindle notes.
- [docs/runtime-progress.md](docs/runtime-progress.md) explains release-build script execution, ETA measurement, progress display, and inline marker validation.
- [docs/batch-recovery.md](docs/batch-recovery.md) is the detailed checklist for Batch API recovery.
- [CHANGELOG.md](CHANGELOG.md) records release history.

## Quick Start

Inspect the EPUB first. A spine number is the reading-order number of a content file inside the EPUB. `FROM` and `TO` in translation commands are the 1-based spine numbers printed by `inspect-epub.ps1`, not reader page numbers.

```powershell
.\scripts\inspect-epub.ps1 .\book.epub
```

`inspect-epub.ps1` output example:

Use this output to choose the first and last numbers passed to `usage.ps1` or `translate-deepseek.ps1 -From -To`. Compare the chapter title in `toc` with the `Href` column in `inspect`, then choose a number that looks like real body text. For example, if the table of contents points chapter 1 to `c66.xhtml`, and `inspect` shows `c66.xhtml` as `No 9`, use `9 9` for the check range.

```text
[inspect]
  No  Linear  Exists   Bytes  Blocks  Media Type              Href
-------------------------------------------------------------------
   7  yes     yes       4658       8  application/xhtml+xml   c56.xhtml
   8  yes     yes       3249       8  application/xhtml+xml   c5P.xhtml
   9  yes     yes       4246       8  application/xhtml+xml   c66.xhtml

[toc]
- Preface -> c56.xhtml
- Why You Should Not Stop Reading Here -> c5P.xhtml
- 1 Writing Is a Trade -> c66.xhtml
```

Create glossary candidate files next to the EPUB. If `book.json` already exists, the translation scripts will use it automatically.

```powershell
.\scripts\create-glossary.ps1 .\book.epub
```

Set the DeepSeek API key.

```powershell
$env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput
```

Next, check usage and run a trial translation for one selected content file. In the usage check, review the estimated request count and input / output tokens for the selected range. In the trial translation, translate only the selected content file before the full conversion and confirm that body text is translated into Japanese, XHTML structure such as headings, links, and emphasis is preserved, and glossary terms are applied.

The check range is required. Check the `inspect-epub.ps1` output and choose a number that looks like real body text for your EPUB. In the example below, `9 9` means both the first and last numbers are `9`, so only the ninth content file is checked. Change it to match the actual EPUB.

Check usage. This command does not call the provider.

```powershell
.\scripts\usage.ps1 .\book.epub 9 9 -Provider deepseek
```

`usage.ps1` output example:

It estimates how many requests and tokens would be used to translate the selected range. For an uncached range, estimated tokens are printed. If every selected block is already cached, the uncached estimate can be `0`.

```text
Usage estimate only. No translation provider was called.
Provider: deepseek
Model: deepseek-v4-flash
Pages: 1/50 selected
Blocks: 8 total, 0 cached, 8 uncached
Source chars: 3594 total, 3594 uncached
Estimated API requests: 8
Estimated tokens: input 2035, output 902, total 2937
Note: token counts are approximate before the API returns actual usage.
```

How to read it:

```text
Blocks: 8 total, 0 cached, 8 uncached
```

The selected range contains 8 translatable blocks, and all 8 are uncached. Only uncached blocks are sent to the API.

```text
Estimated API requests: 8
Estimated tokens: input 2035, output 902, total 2937
```

Translating this range for the first time is estimated to use 8 requests and about 2937 total tokens. Token counts are approximate before the provider returns actual usage.

Run a trial translation for one content file. This command calls the provider and writes `.\book_jp.epub`. Use `-From` and `-To` to limit the range.

```powershell
.\scripts\translate-deepseek.ps1 .\book.epub -From 9 -To 9
```

Trial translation output example:

This is the result of translating only the selected content file.

```text
Done.
Output: .\book_jp.epub
Translation:
  provider: deepseek
  model: deepseek-v4-flash
  pages translated: 1
  blocks translated: 8
Cache:
  hits: 0
  misses: 8
  writes: 8
```

How to read it:

```text
pages translated: 1
blocks translated: 8
```

One selected content file was processed, and 8 blocks were translated.

```text
hits: 0
misses: 8
writes: 8
```

There were 0 existing cache hits, 8 uncached blocks were sent to the API, and 8 successful translations were written to cache. On a rerun, `hits` should increase while `misses` and `writes` decrease.

Argument forms for usage estimate and trial translation. `usage.ps1` requires the first and last numbers; `translate-deepseek.ps1` treats `-From` / `-To` as optional (omitting them runs a full conversion).

```text
usage.ps1                <input.epub> <from> <to> -Provider deepseek
translate-deepseek.ps1   <input.epub> [-From <from> -To <to>]
```

Run the full conversion. Without `-From` / `-To` the entire spine is processed. The output is written next to the input as `<input>_jp.epub`.

```powershell
.\scripts\translate-deepseek.ps1 .\book.epub
```

If the summary prints an untranslated report or recovery log, recover only the missing blocks.

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek
```

The daily scripts run `cargo run --release -- ...`, set the standard cache root, detect a same-name glossary, and keep command lines short. Use debug `cargo run -- ...` commands only for short development checks.

For local Ollama testing, use the dedicated script:

```powershell
.\scripts\translate-ollama.ps1 .\book.epub
```

Useful DeepSeek scripts:

```powershell
.\scripts\rebuild-from-cache.ps1 .\book.epub
.\scripts\rebuild-from-cache.ps1 .\book.epub fixed
.\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Manual .\book.manual.json
```

## Detailed Script Examples

The local Ollama script runs `cargo run --release -- ...`, sets `EPUBICUS_*` environment variables, and writes the output next to the input with `_jp` appended to the file name:

```text
.\book.epub -> .\book_jp.epub
```

For detailed Ollama modes, see [docs/translation-workflow.ja.md](docs/translation-workflow.ja.md).

For OpenAI Batch API runs, use the matching Batch script:

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\translate-openai-batch.ps1 .\book.epub
```

It also writes `.\book_jp.epub` next to the input file. Use a page range while
checking cost and quality:

```powershell
.\scripts\translate-openai-batch.ps1 .\book.epub -From 3 -To 3
```

To check or resume without immediately running:

```powershell
. .\scripts\translate-openai-batch.ps1 .\book.epub -NoRun
Invoke-EpubicusOpenAiBatchStatus
Invoke-EpubicusOpenAiBatchVerify
Invoke-EpubicusOpenAiBatch
```

For normal OpenAI API, Claude API, or DeepSeek API runs, use the provider-specific scripts:

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
.\scripts\translate-openai.ps1 .\book.epub

$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
.\scripts\translate-claude.ps1 .\book.epub

$env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput
.\scripts\translate-deepseek.ps1 .\book.epub
```

These scripts support the same page-range and usage-estimate options:

```powershell
.\scripts\translate-openai.ps1 .\book.epub -From 3 -To 3
.\scripts\translate-openai.ps1 .\book.epub -From 3 -To 3 -UsageOnly
.\scripts\translate-claude.ps1 .\book.epub -From 3 -To 3
.\scripts\translate-claude.ps1 .\book.epub -From 3 -To 3 -UsageOnly
.\scripts\translate-deepseek.ps1 .\book.epub -From 3 -To 3
.\scripts\translate-deepseek.ps1 .\book.epub -From 3 -To 3 -UsageOnly
```

For macOS/Linux shells, use the `.sh` versions instead:

```bash
scripts/translate-ollama.sh ./book.epub

export OPENAI_API_KEY="..."
scripts/translate-openai.sh ./book.epub --from 3 --to 3 --usage-only

export ANTHROPIC_API_KEY="..."
scripts/translate-claude.sh ./book.epub --from 3 --to 3 --usage-only

export DEEPSEEK_API_KEY="..."
scripts/translate-deepseek.sh ./book.epub --from 3 --to 3 --usage-only

export OPENAI_API_KEY="..."
scripts/translate-openai-batch.sh ./book.epub --from 3 --to 3
```

See [docs/operation-guide.ja.md](docs/operation-guide.ja.md) for a practical
Japanese workflow guide covering local Ollama, normal OpenAI/Claude/DeepSeek API runs,
OpenAI Batch API runs, cache recovery, and cost checks.
Check OpenAI API usage at <https://platform.openai.com/usage> and billing at
<https://platform.openai.com/settings/organization/billing/overview>.
Multilingual input/output support is planned in
[docs/multilingual-design.md](docs/multilingual-design.md).

Translation results are cached per-input EPUB under an OS-standard cache root (Windows: `%LOCALAPPDATA%\epubicus\cache`, Unix: `~/.cache/epubicus`). Each input gets its own subdirectory named after the SHA-256 hash of its bytes, with `manifest.json` and `translations.jsonl` inside.

Provider responses are validated before they are written to the cache. Empty responses, unchanged English source text, prompt-wrapper leaks, missing/changed/added inline placeholders, and likely refusal/explanation text are retried according to `--retries`. Added bracket-style markers such as `⟦/S1⟧` or `⟦DAX⟧` are rejected so tag-restoration markers do not leak into the EPUB output. If a likely refusal/explanation still fails after retries and `--fallback-provider` is set, the original source text is translated again with the fallback provider. If the fallback also fails, the run stops without caching the bad response.

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

After an interrupted run, rerun the same `translate` command to resume from uncached blocks. Because the cache directory is keyed by input EPUB hash, resuming works regardless of the output path. Use the same provider, model, style, and glossary when resuming because those settings are part of each block cache key. During parallel execution, each successful block is written to the cache immediately instead of waiting for the whole page batch to finish, so an interruption only loses blocks that were still in flight and had not returned yet. The progress bar starts at the cached position and shows a message such as `resuming: 991/5805 cached`. The final summary shows the cache location and a `partial rebuild` command for assembling an EPUB from the current cache.

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
  --cache-root .\.cache `
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
  --cache-root .\.cache `
  --provider ollama `
  --model qwen3:14b `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

The default cache root is shared across providers (e.g. `.cache/`). The cache key includes the provider, so multiple providers can coexist in the same cache directory without collisions.

If any block is written unchanged, epubicus writes `recovery\<output EPUB name>\untranslated.txt` under the cache directory. Use that file after noisy runs to inspect the page number, XHTML href, reason, and original source block for each untranslated output block. Because it lives with the cache, `cache clear` / `cache prune` can clean translation cache, batch artifacts, and recovery logs together.

epubicus also writes `recovery\<output EPUB name>\recovery.jsonl`. The exact path is printed at the end of `translate` as `Recovery log:`. Use the `recover` subcommand to retry only those blocks and insert successful translations back into the normal cache.

```powershell
$log = ".\.cache\0123456789abcdef0123456789abcdef\recovery\book_jp\recovery.jsonl"
cargo run -- recover $log
```

To recover and rebuild in one step, pass `--rebuild`. When every selected item is recovered, epubicus rebuilds the EPUB from cache with `--partial-from-cache` and the output path recorded in the recovery log.

```powershell
cargo run -- recover $log --rebuild
```

You can also recover from the newest recovery log for an input EPUB with the helper script. With `-Provider deepseek`, it uses the shared `.cache/` directory and automatically picks up `book.json` next to `book.epub` as the glossary.

```powershell
.\scripts\recover-from-cache.ps1 .\book.epub `
  -Provider deepseek
```

To scan an existing output EPUB and immediately recover suspicious untranslated blocks, use:

```powershell
.\scripts\scan-and-recover.ps1 .\book.epub `
  -Provider deepseek
```

Do not put spaces after the PowerShell line-continuation backtick.

Pass `--output` to write the rebuilt EPUB elsewhere. To rebuild manually instead, run:

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.cache `
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

`translate` creates an EPUB and shows a progress bar with elapsed time, ETA, selected spine pages, translatable XHTML block count, and in-flight provider request progress for uncached blocks. ETA is measured from the current run or resume point, but spine pages 1-3 are excluded from ETA timing and character totals. epubicus counts uncached source characters from spine page 4 onward at startup, keeps ETA as pending until provider work on page 4 or later has been measured for at least five minutes, then projects the remaining uncached characters from that cumulative rate. Cached work from previous runs is shown in the progress position but is not included in the ETA denominator. When the provider returns usage data, such as OpenAI or Claude, the final summary includes API request count and input / output / total tokens.

`test` prints translated text for a selected spine range to stdout. It does not create an EPUB.

`inspect` shows OPF path, spine order, `linear` state, referenced file existence, file size, and a rough count of translatable XHTML blocks.

`toc` shows EPUB3 `nav.xhtml` or EPUB2 NCX table-of-contents entries with indentation and target hrefs.

`glossary` extracts candidate proper nouns and terms into JSON for manual review.

`unlock` removes a stale input-use flag for an EPUB. Without `--force`, it only removes the flag when the recorded process is no longer running on the same host.

`batch prepare` creates local OpenAI Batch API request artifacts without making a network call. It writes compatibility `requests.jsonl` plus `requests.part-0001.jsonl` style part files; `--max-requests-per-file <N>` defaults to `50000` and `--max-bytes-per-file <N>` defaults to `200000000`. `batch run` orchestrates prepare, submit, status polling, fetch, import, health, and verify; without `--wait`, it stops after the current remote status if the batch is still running, so the same command can be re-run later. `batch submit` uploads each request part and creates one OpenAI Batch API job per part. `batch status` refreshes all remote part statuses into `batch_manifest.json`. `batch fetch` downloads missing part output/error files, reuses existing part files on rerun, and rebuilds aggregate `output.jsonl` and `remote_errors.jsonl` files. `batch import` imports the fetched `output.jsonl` into the normal translation cache, marks fetched remote error lines as `failed`, and reports `imported_with_errors` if any item failed or was rejected; already-cached identical output is reported separately and imports can be rerun. `batch retry-requests` writes `retry_requests.jsonl` for failed/rejected uncached items without submitting it. `--output <PATH>` can import another local Batch API output JSONL file. `batch reroute-local` marks selected unfinished items as `local_pending`. `batch translate-local` translates `local_pending` items through the normal provider backend and writes them to the original batch cache slots. Local fallback and retry-planning commands support `--limit <N>` and `--priority page-order|failed-first|hard-first|short-first|oldest-first` for bounded catch-up runs. `batch health` shows the local batch manifest, remote batch IDs, per-part remote status counts, work item states, request count, import report, cache-backed work, and oldest pending age. `batch verify` checks the current EPUB, `work_items.jsonl`, and cache for missing, stale, orphaned, conflicting, or invalid entries.

One-command Batch API flow:

```powershell
$env:OPENAI_API_KEY = "..."
cargo run -- batch run .\book.epub --provider openai --model gpt-5-mini --wait --poll-secs 180 --output .\book.ja.epub
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
| `--from N` | first content file | First content-file number to translate, as printed by `inspect-epub.ps1` |
| `--to N` | last content file | Last content-file number to translate, as printed by `inspect-epub.ps1` |
| `--partial-from-cache` | false | Replace cache hits with translations and keep cache misses unchanged. If untranslated blocks remain, write the EPUB and report, then exit with an error |

When an EPUB and recovery log were written but untranslated blocks remain, `recover` leaves unrecoverable items in `failed.jsonl`, or `scan-recovery` detects suspicious untranslated blocks and writes a recovery log, epubicus exits with code `2` for a recoverable error. Non-recoverable failures such as invalid input EPUBs or unwritable output paths use the normal error code `1`.

### `test`

| Option | Default | Description |
|--|--|--|
| `--from N` | required | First content-file number to print, as printed by `inspect-epub.ps1` |
| `--to N` | required | Last content-file number to print, as printed by `inspect-epub.ps1` |

### Shared `translate` / `test` Options

CLI arguments take precedence over environment variables.

| Option | Environment variable | Default | Description |
|--|--|--|--|
| `-p, --provider ollama\|openai\|claude\|deepseek` | `EPUBICUS_PROVIDER` | `ollama` | Translation provider |
| `-m, --model NAME` | `EPUBICUS_MODEL` | provider-specific | Model name |
| `--fallback-provider ollama\|openai\|claude\|deepseek` | `EPUBICUS_FALLBACK_PROVIDER` | none | Fallback provider used only when the primary provider returns a likely refusal/explanation and retries are exhausted |
| `--fallback-model NAME` | `EPUBICUS_FALLBACK_MODEL` | fallback-provider-specific | Model name for the fallback provider |
| `--ollama-host URL` | `EPUBICUS_OLLAMA_HOST` | `http://localhost:11434` | Ollama endpoint |
| `--openai-base-url URL` | `EPUBICUS_OPENAI_BASE_URL` | `https://api.openai.com/v1` | OpenAI API base URL |
| `--claude-base-url URL` | `EPUBICUS_CLAUDE_BASE_URL` | `https://api.anthropic.com/v1` | Claude / Anthropic API base URL |
| `--openai-api-key KEY` | `OPENAI_API_KEY` | none | OpenAI API key. `--openai-api-key` takes precedence |
| `--anthropic-api-key KEY` | `ANTHROPIC_API_KEY` | none | Anthropic API key. `--anthropic-api-key` takes precedence |
| none | `DEEPSEEK_API_KEY` | none | DeepSeek API key. You can also use `--prompt-api-key` |
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
.\scripts\scan-and-recover.ps1 .\book.epub `
  -Provider deepseek
```

Provider-specific `--model` defaults:

| Provider | Default model |
|--|--|
| `ollama` | `qwen3:14b` |
| `openai` | `gpt-5-mini` |
| `claude` | `claude-sonnet-4-5` |
| `deepseek` | `deepseek-v4-flash` |

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

DeepSeek uses its Anthropic-compatible Messages API. Set `DEEPSEEK_API_KEY` or use `--prompt-api-key`:

```powershell
$env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput
cargo run -- test .\book.epub --from 1 --to 1 --provider deepseek --model deepseek-v4-flash
```

To use the PowerShell script:

```powershell
$env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput
.\scripts\translate-deepseek.ps1 .\book.epub -From 3 -To 3 -UsageOnly
.\scripts\translate-deepseek.ps1 .\book.epub -From 3 -To 3
```

Interactive key prompt:

```powershell
cargo run -- test .\book.epub --from 1 --to 1 --provider openai --prompt-api-key
cargo run -- test .\book.epub --from 1 --to 1 --provider claude --prompt-api-key
cargo run -- test .\book.epub --from 1 --to 1 --provider deepseek --prompt-api-key
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

`source_lang` is filled from the source EPUB's `dc:language` metadata. If it is missing, epubicus writes `auto`. Edit `dst` values:

```json
{
  "source_lang": "la",
  "target_lang": "ja",
  "entries": [
    {
      "src": "Caesar",
      "dst": "カエサル"
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
- Ollama `/api/chat`, OpenAI `/responses`, Claude `/messages`, and DeepSeek Anthropic-compatible `/messages` translation.
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
