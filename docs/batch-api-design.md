# Batch API mode design

Last updated: 2026-04-30

This document designs a future batch translation mode for epubicus. It is a
planning document only; no batch implementation is assumed to exist yet.

## Goals

- Translate large EPUBs through an asynchronous API batch workflow.
- Split the workflow into visible, resumable stages: prepare, submit, status,
  fetch, import, and assemble.
- Reuse the existing cache as the canonical translation store.
- Keep the normal `translate` path available for immediate runs and for
  completing failed batch items.
- Make partial failure explicit and retryable without losing successful work.

## Non-goals

- Do not make batch mode replace the current synchronous provider flow.
- Do not write batch output directly into XHTML.
- Do not rely on output JSONL order.
- Do not require a batch job to finish during a single epubicus process.
- Do not implement browser or ChatGPT UI automation.

## Relevant OpenAI Batch API facts

OpenAI Batch API accepts a JSONL file uploaded with `purpose=batch`, then creates
a batch with an endpoint such as `/v1/responses` and `completion_window: "24h"`.
Each JSONL line has a unique `custom_id`, `method`, `url`, and request `body`.
The output order is not guaranteed to match input order, so epubicus must map
results by `custom_id`. Successful responses are available from
`output_file_id`; failed requests are available from `error_file_id`.

Source references:

- https://platform.openai.com/docs/guides/batch/
- https://platform.openai.com/docs/api-reference/batch/retrieve

## High-level workflow

```text
EPUB
  -> batch prepare
  -> requests.jsonl + batch_manifest.json
  -> batch submit
  -> OpenAI batch id
  -> batch status
  -> batch fetch
  -> output.jsonl / errors.jsonl
  -> batch import
  -> translations.jsonl cache
  -> translate / partial-from-cache
  -> EPUB output
```

The batch path should fill the same cache used by normal translation. Assembly
should then reuse the existing EPUB rewrite path.

## Proposed command surface

```powershell
epubicus batch prepare .\book.epub -p openai -m gpt-5-mini
epubicus batch submit .\book.epub
epubicus batch status .\book.epub
epubicus batch fetch .\book.epub
epubicus batch import .\book.epub
epubicus translate .\book.epub -o .\book.ja.epub --partial-from-cache
```

Optional convenience command, later:

```powershell
epubicus batch run .\book.epub -p openai -m gpt-5-mini
```

`batch run` should only orchestrate the same subcommands. The individual stages
remain the primary recovery and debugging interface.

## Batch workspace layout

Store batch artifacts separately from the translation cache but key them by the
same input EPUB hash.

```text
<cache-root>/<input-hash>/
  manifest.json
  translations.jsonl
  batch/
    batch_manifest.json
    work_items.jsonl
    requests.jsonl
    retry_requests.jsonl
    local_requests.jsonl
    output.jsonl
    errors.jsonl
    rejected.jsonl
    import_report.json
```

Rationale:

- The cache remains the durable source of successful translations.
- Batch artifacts are inspectable and can be deleted independently.
- Reusing `<input-hash>` preserves resume behavior when output paths change.

## Concurrency and locking requirements

Multiple epubicus processes may run at the same time. They may target different
EPUBs, or they may accidentally target the same input EPUB and cache directory.
The implementation must guarantee that concurrent processes do not corrupt cache
or batch state.

Lock scope:

| Lock | Path | Protects |
|--|--|--|
| input run lock | OS temp dir `epubicus/.locks/<input-hash>.<path-hash>.run.lock` | Prevents multiple commands from reading or processing the same input EPUB path at the same time |
| cache lock | `<cache-root>/.locks/<input-hash>.lock` | `translations.jsonl`, `manifest.json`, cache directory deletion |
| batch lock | `<cache-root>/.locks/<input-hash>.batch.lock` | `batch_manifest.json`, `work_items.jsonl`, request/output/error artifacts |

Rules:

- Processes for different input hashes must not block each other.
- A command that reads an input EPUB should acquire the input run lock before
  unpacking or scanning it. If the run lock already exists, the command should
  fail immediately instead of waiting. This prevents accidental double starts
  for the same EPUB.
- If the input run lock records the current host and a process id that is no
  longer running, a later command may remove that stale run lock automatically
  and continue.
- `unlock <INPUT.epub>` should explicitly remove a stale input run lock.
  `unlock --force <INPUT.epub>` may remove a run lock even when the recorded
  process still appears active, and should be documented as a last-resort
  recovery command.
- All writes to `translations.jsonl` must hold the cache lock.
- All writes to `manifest.json` must hold the cache lock.
- `finalize_completion` must hold the cache lock and must not delete a cache
  directory while another process is using it.
- All writes to batch artifacts must hold the batch lock.
- A command that imports batch output into cache must acquire locks in a stable
  order: batch lock first, then cache lock. No command may acquire them in the
  opposite order.
- Locks should be advisory file locks where supported. The lock file should
  record process id, hostname, command, and timestamp for diagnostics.
- Commands should default to waiting for locks with a visible message. A later
  `--lock-timeout` option can limit the wait.

Lock acquisition policy:

- Default behavior is wait-and-report. While waiting, print a compact status at
  a fixed interval with the lock path, holder process id, holder command,
  elapsed wait time, and last heartbeat time.
- `--lock-timeout <seconds>` should fail cleanly after the timeout with the same
  holder metadata and a recovery hint.
- `--lock-timeout 0` should mean fail immediately if the lock is held. This is
  useful for scheduled jobs and wrapper scripts that should not block.
- Lock files must include `pid`, `hostname`, `command`, `purpose`,
  `input_hash`, `created_at`, and `heartbeat_at`.
- Long-running commands should refresh `heartbeat_at` periodically while they
  own the lock.

Stale lock policy:

- A fresh lock must never be removed by another process.
- A lock is only stale when the heartbeat is older than the configured stale
  threshold and, on the same host, the recorded process id is no longer alive.
- If process liveness cannot be checked, heartbeat age is only diagnostic unless
  the user explicitly asks to break the stale lock.
- Breaking a stale lock should require an explicit option such as
  `--break-stale-lock`. The command should print the old holder metadata before
  replacing the lock.
- Recovery instructions should prefer rerunning the interrupted command first.
  Manual lock deletion is a last resort.

Deadlock avoidance:

- Any command that needs both locks must acquire the batch lock first and the
  cache lock second.
- No command may wait for the batch lock while holding the cache lock.
- Commands must not hold locks while waiting for user input.
- Commands should not hold locks while waiting for long remote Batch API
  completion. They should lock only to read or update local state, release the
  lock during remote waiting or polling delays, then reacquire before committing
  local changes.
- `batch import` is the narrow critical section: acquire batch lock, acquire
  cache lock, validate output against `work_items.jsonl`, append accepted cache
  entries, update the ledger, then release both locks.

Read behavior:

- Read-only commands may read without a lock when they can tolerate a changing
  snapshot, but they must re-read after acquiring a lock before making a write.
- Commands that display authoritative state, such as a final import summary,
  should read under the relevant lock or read immediately after the locked
  update.
- A lock-free read must treat partial or malformed in-progress files as
  transient and retry or report a clear "state is being updated" message.

Atomic file update rules:

- JSON state files such as `batch_manifest.json`, `work_items.jsonl`,
  `import_report.json`, and `manifest.json` should be written to a temporary
  file in the same directory, flushed, and atomically renamed into place.
- Append-only files such as `translations.jsonl` may append under lock. Duplicate
  cache keys are allowed only if the existing translated text is identical; a
  conflicting duplicate must be reported and must not overwrite silently.
- `read_cache_entries` should continue treating the last valid entry for a key
  as the in-memory value only after duplicate conflict rules are defined. The
  safer batch import behavior is to check the loaded cache before appending.

Concurrent command behavior:

| Scenario | Required behavior |
|--|--|
| Two `translate` commands for same EPUB | Both may run, but cache writes are serialized; duplicate successful keys are ignored if identical |
| `translate` while `batch import` runs | Cache writes are serialized; neither process may truncate or delete cache under the other |
| `batch status` while `batch import` runs | Status may wait for the batch lock before updating local manifest |
| `batch prepare --clear-cache` while another process uses cache | Must wait or fail with a clear lock message; must not delete active cache |
| Stale lock left by a killed process | Must report holder metadata; may break only with explicit stale-lock option |
| Long `batch submit`/`batch fetch` polling | Must not block unrelated cache writes while sleeping or waiting remotely |
| Different EPUB hashes | No shared locks; run independently |

The state ledger makes concurrency safer but does not replace locking. State
updates and cache writes must still be serialized.

## Manifest shape

`batch_manifest.json` should track both local artifacts and remote IDs.

```json
{
  "schema_version": 1,
  "input_sha256": "...",
  "provider": "openai",
  "model": "gpt-5-mini",
  "endpoint": "/v1/responses",
  "completion_window": "24h",
  "created_at": "2026-04-30T00:00:00Z",
  "updated_at": "2026-04-30T00:00:00Z",
  "request_file": "requests.jsonl",
  "work_items_file": "work_items.jsonl",
  "request_count": 5798,
  "file_id": null,
  "batch_id": null,
  "status": "prepared",
  "output_file_id": null,
  "error_file_id": null,
  "output_file": null,
  "error_file": null,
  "imported_count": 0,
  "failed_count": 0,
  "rejected_count": 0
}
```

## Work item state ledger

Batch mode needs a local state ledger in addition to the remote batch object.
The remote batch status answers "what happened to this OpenAI batch"; the local
ledger answers "what remains for this EPUB".

`work_items.jsonl` should contain one row per translatable uncached unit at
prepare time. Each row should be append-friendly or rewritable through an atomic
rewrite, but the logical model is a table keyed by `custom_id` and `cache_key`.

Example:

```json
{
  "custom_id": "epubicus:<input_hash>:p0008:b0037:<cache_key>",
  "cache_key": "...",
  "page_index": 8,
  "block_index": 37,
  "source_hash": "...",
  "source_chars": 682,
  "provider": "openai",
  "model": "gpt-5-mini",
  "state": "prepared",
  "attempt": 1,
  "last_error": null,
  "updated_at": "2026-04-30T00:00:00Z"
}
```

Required states:

| State | Meaning |
|--|--|
| `cached` | A valid translation is already in `translations.jsonl` |
| `prepared` | Local request exists but has not been submitted |
| `submitted` | Request belongs to a submitted remote batch |
| `completed` | Remote output line exists but is not imported yet |
| `failed` | Remote error line exists |
| `rejected` | Remote output exists but validation failed |
| `imported` | Valid translation was written to cache |
| `local_pending` | Item has been reassigned to local translation |
| `local_imported` | Local translation was written to cache |
| `skipped` | User intentionally left the original text |

The ledger is the basis for reporting and rerouting. A command should be able to
select items by state and emit a new request file for either another batch or a
local provider run.

## Request JSONL design

Use one request per uncached translation block. Long blocks that the normal path
would split should be split before preparing batch requests so each request maps
to a cacheable unit.

For `/v1/responses`, a request line should be shaped like this:

```json
{
  "custom_id": "epubicus:<input_hash>:p0008:b0037:<cache_key>",
  "method": "POST",
  "url": "/v1/responses",
  "body": {
    "model": "gpt-5-mini",
    "instructions": "...system prompt...",
    "input": "...user prompt..."
  }
}
```

`custom_id` must be unique within a batch and must be enough to recover:

- input EPUB hash
- page number or spine index
- block index within the page
- cache key

The cache key is the most important part for import. Page and block identifiers
are for diagnostics and retry reports.

## Stage behavior

### `batch prepare`

Responsibilities:

- Read the EPUB and selected page range.
- Extract translatable blocks exactly like `translate`.
- Compute glossary subset and cache key for each block.
- Skip valid cache hits.
- Write uncached requests to `requests.jsonl`.
- Write all selected work items and their initial states to `work_items.jsonl`.
- Write or update `batch_manifest.json`.
- Print a summary: selected pages, total blocks, cached blocks, request count,
  estimated source chars, and target artifact path.

Validation:

- Refuse `--no-cache`; batch mode depends on cache import.
- Respect `--clear-cache` before scanning.
- Use the same provider/model/style/glossary inputs as cache keys.
- Do not call the remote provider.

### `batch submit`

Responsibilities:

- Read `batch_manifest.json`.
- Upload `requests.jsonl` through Files API with `purpose=batch`.
- Create a batch for `/v1/responses` with `completion_window: "24h"`.
- Save `file_id`, `batch_id`, and initial status.

Validation:

- Refuse submit when request count is zero.
- Refuse submit if a live `batch_id` already exists unless `--force` is used.
- Confirm the input file exists and matches the manifest.

### `batch status`

Responsibilities:

- Retrieve the remote batch object by `batch_id`.
- Update local status and file IDs.
- Print status, request counts, timestamps, and output/error file IDs when
  available.
- Optionally print local ledger counts by state, such as `imported`,
  `submitted`, `failed`, `rejected`, and `local_pending`.

Statuses to handle:

- `validating`
- `in_progress`
- `finalizing`
- `completed`
- `failed`
- `expired`
- `cancelling`
- `cancelled`

### `batch fetch`

Responsibilities:

- Retrieve status first.
- If `output_file_id` exists, download it to `output.jsonl`.
- If `error_file_id` exists, download it to `errors.jsonl`.
- Preserve previous downloads unless `--force` is used.

Notes:

- A cancelled batch may have partial output.
- Fetch should not import automatically.

### `batch import`

Responsibilities:

- Read `output.jsonl`.
- Map each line by `custom_id`.
- Extract translated text from the response body.
- Run the same translation validation used by normal provider responses.
- Write valid translations to `translations.jsonl` immediately.
- Update `work_items.jsonl` states as lines are imported, failed, or rejected.
- Write invalid or refusal-like responses to `rejected.jsonl`.
- Read `errors.jsonl` and write failed request metadata into the import report.
- Optionally create `retry_requests.jsonl` for failed/rejected uncached items.

Validation:

- Never trust output order.
- Never cache invalid translations.
- Treat missing `custom_id` as an import error.
- Treat duplicate `custom_id` as an import error unless the duplicate maps to an
  identical translation.

### `batch reroute-local`

Responsibilities:

- Read `work_items.jsonl`.
- Select items in states such as `prepared`, `failed`, `rejected`, or
  `submitted` from an expired/cancelled batch.
- Mark them as `local_pending`.
- Write `local_requests.jsonl` or call the existing synchronous translation
  backend directly, depending on the selected mode.
- Preserve `custom_id` and `cache_key` so local results import into the same
  cache slots.

Possible command forms:

```powershell
epubicus batch reroute-local .\book.epub --state failed --state rejected --provider ollama --model qwen3:14b
epubicus batch reroute-local .\book.epub --remaining --provider ollama --model qwen3:14b
```

`--remaining` should mean every item that is not `cached`, `imported`,
`local_imported`, or `skipped`.

Current implementation:

- Supports repeated `--state <STATE>`.
- Supports `--remaining`.
- Supports `--endgame-threshold <N>`.
- Supports `--limit <N>` for bounded rerouting.
- Supports `--priority page-order|failed-first|hard-first|short-first|oldest-first`.
- Excludes items that are already imported, locally imported, or already present
  in cache from remaining selection.
- Marks selected items as `local_pending`; provider execution remains in the
  separate `batch translate-local` step.

### `batch translate-local`

Responsibilities:

- Translate `local_pending` items through the normal provider backend, usually
  Ollama.
- Write successful results to the cache immediately.
- Mark successful items as `local_imported`.
- Leave failures as `local_pending` with `last_error` populated.

Current implementation:

- Processes `local_pending` items through the existing `Translator` backend.
- Writes successful results to the original batch `cache_key`, preserving the
  batch cache slot even when the local provider/model differs.
- Marks already cached pending items as `local_imported` without another
  provider call.
- Supports `--limit <N>` for bounded local catch-up runs.
- Supports `--priority page-order|failed-first|hard-first|short-first|oldest-first`
  so bounded runs can finish easier or more urgent items first.

This command is intentionally separate from `batch reroute-local` so the user
can inspect the selected work before spending local compute time.

### `translate --partial-from-cache`

Responsibilities:

- Use existing cache hits to assemble an EPUB.
- Leave misses as original source when explicitly requested.
- For full final output, run normal `translate` without `--partial-from-cache`
  after import; remaining misses can be translated synchronously or with local
  fallback.

## Failure and recovery model

Batch mode should classify every request into one of these states:

| State | Meaning | Next action |
|--|--|--|
| cached | Valid translation already in `translations.jsonl` | Skip |
| prepared | Request exists locally, not submitted | Submit |
| submitted | Remote batch has accepted the file | Wait/status |
| completed | Output line exists | Import and validate |
| failed | Error line exists | Add to retry file |
| rejected | Output exists but validation failed | Retry or local fallback |
| imported | Valid translation written to cache | Assemble |
| local_pending | Item was reassigned to local translation | Run `batch translate-local` |
| local_imported | Local translation was written to cache | Assemble |
| skipped | User intentionally kept source text | Assemble partial output |

This avoids relying on a single "last processed" pointer, which is unsafe for
parallel and asynchronous work.

## P2P-inspired speed and safety techniques

Batch mode can borrow useful ideas from P2P clients without becoming a P2P
system. The useful abstraction is "verified pieces": each EPUB block or chunk is
an independently tracked work item with hashes, state, priority, and retry
history.

### Speed techniques

| Technique | How it applies to epubicus | Benefit |
|--|--|--|
| Piece scheduling | Treat each block/chunk as a schedulable piece in `work_items.jsonl` | Allows retries, local rerouting, and progress reporting by item |
| Priority queue | Sort by state, failure count, size, placeholder complexity, or page order | Avoids leaving difficult pieces until the end |
| Endgame mode | When remaining count falls below a threshold, reroute leftovers to local or synchronous API | Avoids waiting for a long batch cycle for a few items |
| Duplicate work absorption | If two processes produce the same cache key, accept the first valid result and ignore identical duplicates | Makes concurrent runs less wasteful and safer |
| Request packing limits | Split JSONL batches by request count and file size before API limits are reached | Keeps large EPUB jobs submit-safe |
| Health summary | Show counts by state and stale age, not only remote batch status | Makes bottlenecks visible |

Suggested priority modes:

| Mode | Behavior |
|--|--|
| `failed-first` | Retry failed/rejected items before untouched items |
| `hard-first` | Prioritize long items and placeholder-heavy items |
| `short-first` | Fill many small items quickly for visible progress |
| `page-order` | Preserve reading order for easier manual inspection |
| `oldest-first` | Process items with the oldest `updated_at` first |

### Safety techniques

| Technique | How it applies to epubicus | Benefit |
|--|--|--|
| Source hash | Store hash of encoded source text per work item | Detects stale work when extraction changes |
| Prompt hash | Store hash of system/user prompt per request | Prevents importing output produced by a different prompt |
| Translation hash | Store hash of accepted translated text | Helps detect duplicate/conflicting imports |
| Recheck command | Re-scan EPUB, cache, and work ledger | Finds missing, stale, orphaned, or conflicting items |
| State reconciliation | Prefer cache truth over stale ledger states | Recovery stays correct after interruption |
| Quarantine | Put invalid outputs in `rejected.jsonl` instead of cache | Prevents bad translations from contaminating assembly |
| Stable lock order | Always acquire batch lock before cache lock when both are needed | Avoids deadlocks |

### Proposed extra commands

```powershell
epubicus batch health .\book.epub
epubicus batch verify .\book.epub
epubicus batch reroute-local .\book.epub --remaining --endgame-threshold 50 --provider ollama
```

`batch health` should print state counts and stale ages, for example:

```text
items: 5798 total | imported 5200 | submitted 300 | failed 20 | rejected 5 | local_pending 10 | remaining 263
last update: 2026-04-30T09:30:12Z
remote batch: in_progress | completed 5300/5798 | failed 20
```

`batch verify` should rebuild expected work from the EPUB and compare it with
cache and `work_items.jsonl`:

| Finding | Meaning | Suggested action |
|--|--|--|
| missing | Expected work item is absent from ledger and cache | Prepare or repair ledger |
| stale | Source/prompt hash differs from ledger | Regenerate request |
| orphaned | Ledger item no longer exists in current EPUB extraction | Ignore or prune |
| cache_conflict | Cache key exists with conflicting translation text | Quarantine and report |
| invalid_cache | Cached translation no longer passes validation | Invalidate and retry |

### Endgame policy

Endgame mode should be explicit at first. A safe default is:

```text
If remaining non-imported items <= N and no active remote batch is expected to finish soon,
reroute those items to local translation or normal synchronous provider translation.
```

The command should show the selected item count before rerouting. Later, an
orchestrated `batch run` command can make this automatic behind a flag.

## Integration with existing cache

The cache remains the only source consumed by EPUB assembly. Batch import should
use the same `CacheRecord` fields:

- `key`
- `translated`
- `provider`
- `model`
- `at`

For batch-imported translations, `provider` should remain `openai` and `model`
should be the batch model. If later local fallback fills a rejected item, the
cache record should reflect the fallback provider/model.

## Interaction with existing features

### Usage estimates

`batch prepare` can reuse usage estimation logic, but it should report that token
counts are approximate until import has actual usage. If OpenAI batch objects
include aggregate usage, `batch status` or `batch fetch` can show it when
available.

### Adaptive concurrency

Adaptive concurrency is not relevant to the remote batch execution itself, but
it remains relevant for fallback synchronous translation after import.

### Local fallback

`batch import` should not call local fallback automatically in the first
implementation. It should emit `rejected.jsonl`, `retry_requests.jsonl`, and
update `work_items.jsonl`. A separate `batch reroute-local` /
`batch translate-local` path can then consume selected states through Ollama or
another local provider.

The local path should be state-based, not file-order-based. For example, the user
should be able to reroute only `failed` and `rejected` items, or every remaining
item after a remote batch expires.

### Glossary

The glossary subset must be computed at prepare time and embedded in each
request prompt. The manifest should store the glossary hash to prevent importing
outputs into a cache generated with different glossary settings.

## Development phases

### Phase 0: Design stabilization

Deliverables:

- This design document.
- Final command names and artifact names.
- Decision on `/v1/responses` as the initial endpoint.
- Decision on `custom_id` format.

Exit criteria:

- Commands and artifact layout are stable enough to implement fixtures.
- Known API constraints are cited in docs.

### Phase 1: Local prepare/import without network

Deliverables:

- Cache and batch file locking primitives.
- `batch prepare` writes `requests.jsonl` and `batch_manifest.json`.
- `batch prepare` writes `work_items.jsonl` with one row per selected uncached
  unit.
- `batch import` can read a local fixture `output.jsonl` and write valid cache
  records.
- Unit tests for `custom_id` parsing, ledger state transitions, duplicate
  handling, validation failure, cache insertion, and lock acquisition.

Exit criteria:

- A fake output fixture can fill cache and `translate --partial-from-cache` can
  assemble from it.
- Two local processes cannot corrupt the same cache or batch workspace.
- No OpenAI API calls are needed for this phase.

### Phase 2: Submit/status/fetch

Deliverables:

- `batch submit` uploads the JSONL file and creates the batch.
- `batch status` retrieves and persists remote status.
- `batch fetch` downloads output and error files. The local output file is
  `output.jsonl`; the remote error download is `remote_errors.jsonl` so it does
  not collide with local import diagnostics in `errors.jsonl`.
- API key handling reuses the existing OpenAI key flow.

Exit criteria:

- A small EPUB can complete a real remote batch round trip.
- Status and fetch can be safely re-run.

### Phase 3: Retry and rejection workflow

Deliverables:

- `batch import` writes `rejected.jsonl`, `errors.jsonl`, and
  `retry_requests.jsonl`.
- `batch prepare --retry-from rejected.jsonl` or equivalent can create a retry
  batch.
- `batch reroute-local` can mark failed, rejected, or remaining items as
  `local_pending`.
- `batch translate-local` can translate `local_pending` items through Ollama and
  write them to cache.

Exit criteria:

- Failed and rejected requests are recoverable without reprocessing successful
  requests.
- The user can switch unfinished remote batch work to local translation without
  manually editing JSONL files.

### Phase 4: Usability and reporting

Deliverables:

- Clear summaries for prepare/status/import.
- `batch health` reports state counts, request counts, cache-backed work items,
  import report counts, remote batch IDs/file IDs/failure counts from the local
  manifest, the oldest pending update, and the age of that pending update.
- `batch verify` reconciles EPUB extraction, cache entries, and
  `work_items.jsonl` in read-only mode. It reports `missing`, `stale`,
  `orphaned`, `cache_conflict`, and `invalid_cache` counts.
- README examples.
- Import report with counts for imported, failed, rejected, duplicate, and
  already-cached items.
- Priority selection for retry/local routing, such as `failed-first`,
  `hard-first`, `short-first`, and `page-order`.
- Optional `batch run` orchestration command.

Exit criteria:

- A user can run the full workflow from README without reading internal docs.

### Phase 5: Hardening

Deliverables:

- Large-file splitting when request JSONL approaches API limits.
- Versioned manifest migrations.
- More response-shape tests.
- Source hash, prompt hash, and translation hash checks for stale or conflicting
  work.
- Explicit endgame mode for rerouting the last N unfinished items.
- Recovery tests for interrupted submit/fetch/import.
- Recovery tests for remote-to-local rerouting after failed, expired, or
  cancelled batches.
- Stress tests for concurrent `translate`, `batch import`, and `batch status`
  processes targeting the same input hash.

Exit criteria:

- Batch mode is safe for large EPUBs and repeated interrupted runs.

## Open questions

- Should `batch prepare` support page ranges from day one?
- Should long-block splitting create separate cache records or an aggregate
  parent record?
- Should import require exact manifest provider/model/glossary match by default?
- Should `batch run` wait and poll, or should it stop after submit?
- Should rejected items be eligible for automatic Ollama fallback during import,
  or only through a separate explicit command?
- Should `batch translate-local` reuse adaptive concurrency for local providers,
  or force a conservative default?
- What should the default endgame threshold be?
- Should priority mode default to `page-order` for readability or `hard-first`
  for earlier risk discovery?
- Should `batch verify` be read-only by default with a separate `--repair`
  option?
