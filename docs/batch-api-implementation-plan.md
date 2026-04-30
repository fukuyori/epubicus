# Batch API implementation plan

Last updated: 2026-04-30

This plan turns `docs/batch-api-design.md` into implementation phases. The
order is chosen to keep the cache safe first, then add local-only batch artifact
handling, then add network operations.

## Phase 0: Groundwork and invariants

Goal: make the current cache path safe enough for batch and multi-process use.

Tasks:

- Add cache and batch lock primitives.
- Add a non-waiting input run lock so a second command for the same EPUB fails
  before unpacking or scanning the file.
- Add automatic stale input run lock cleanup when the recorded process is no
  longer running on the same host.
- Add an explicit `unlock <INPUT.epub>` recovery command with `--force` for
  last-resort manual cleanup.
- Define lock order: batch lock first, cache lock second.
- Add lock metadata:
  - process id
  - hostname
  - command
  - purpose
  - input hash
  - created timestamp
  - heartbeat timestamp
- Add lock waiting controls:
  - default wait with periodic holder-status output
  - `--lock-timeout <seconds>`
  - immediate failure with `--lock-timeout 0`
  - explicit stale-lock recovery option
- Add stale-lock detection:
  - heartbeat age check
  - same-host process liveness check
  - no silent lock breaking
- Protect `translations.jsonl`, `manifest.json`, and cache deletion with the
  cache lock.
- Add duplicate cache-key handling:
  - identical translated text: accept as already done
  - conflicting translated text: reject and report
- Add atomic JSON state writer for manifest-like files.
- Add tests for lock acquisition, lock release, wait timeout, stale-lock
  detection, duplicate cache insert, and atomic write.

Verification:

- `cargo test`
- A test with two writers cannot corrupt `translations.jsonl`.
- Cache deletion refuses or waits while another process holds the cache lock.
- A stale lock is reported with holder metadata and is not broken unless the
  explicit recovery option is used.

Exit criteria:

- Existing `translate` behavior still works.
- Cache writes are serialized across processes.
- Two commands that need both batch and cache state acquire locks in the stable
  order and cannot deadlock.

## Phase 1: Work item ledger and prepare

Goal: create a local batch workspace without calling OpenAI.

Tasks:

- Add `batch` subcommand group.
- Implement `batch prepare`.
- Create batch workspace:
  - `batch/batch_manifest.json`
  - `batch/work_items.jsonl`
  - `batch/requests.jsonl`
- Define structs:
  - `BatchManifest`
  - `WorkItem`
  - `WorkState`
  - `BatchRequestLine`
- Generate stable `custom_id`:
  - input hash
  - page index
  - block index
  - cache key
- Store `source_hash` and `prompt_hash`.
- Skip valid cache hits.
- Support selected page ranges if practical; otherwise document full-book only
  for the first slice.

Verification:

- Unit tests for `custom_id` generation and parsing.
- Golden JSONL fixture for `requests.jsonl`.
- `batch prepare` on the minimal EPUB produces deterministic output.

Exit criteria:

- Running `batch prepare` repeatedly is idempotent.
- No network calls are made.
- Prepared request count matches uncached block count.

## Phase 2: Local import from fixtures

Goal: validate the output-to-cache path before adding network calls.

Tasks:

- Implement `batch import --output <PATH>` for local fixture output.
- Parse output JSONL by `custom_id`, never by line order.
- Extract text from `/v1/responses` shaped output.
- Validate translated text with existing validation.
- Write valid translations to cache immediately.
- Update `work_items.jsonl` states:
  - `completed`
  - `imported`
  - `rejected`
  - `failed`
- Write:
  - `rejected.jsonl`
  - `errors.jsonl`
  - `import_report.json`
- Add stale checks using `source_hash` and `prompt_hash`.

Verification:

- Fixture import fills cache.
- Invalid translation is quarantined, not cached.
- Duplicate output lines are handled deterministically.
- Reordered output lines import correctly.
- `translate --partial-from-cache` can assemble from imported cache.

Current coverage:

- `batch_import_writes_valid_output_to_cache`
- `batch_import_accepts_reordered_output`
- `batch_import_rejects_invalid_translation_without_caching`
- `batch_import_reports_duplicate_output_custom_id`

Exit criteria:

- A complete fake batch cycle works offline:
  `prepare -> fixture output -> import -> partial assemble`.

## Phase 3: Health and verify

Goal: make processing status visible and repairable before remote submission.

Tasks:

- Implement `batch health`. Done for local manifest/work-item/cache counts.
- Implement `batch verify`. Done for read-only local EPUB/work-item/cache
  consistency checks.
- `batch health` reports:
  - state counts (implemented)
  - remaining count (partly visible through state counts)
  - last updated timestamp (oldest pending update and age implemented)
  - cache count (implemented as cache-backed work item count)
  - stale item count (covered by `batch verify`; thresholded health warning is
    planned)
- `batch verify` compares:
  - current EPUB extraction (implemented)
  - `work_items.jsonl` (implemented)
  - `translations.jsonl` (implemented through the loaded cache)
- Report:
  - `missing` (implemented)
  - `stale` (implemented)
  - `orphaned` (implemented)
  - `cache_conflict` (implemented)
  - `invalid_cache` (implemented)
- Keep verify read-only by default.
- Defer `--repair` until the read-only report is trusted.

Verification:

- Tests with local ledger/cache fixtures:
  - `batch_health_reports_local_workspace_state`
  - `batch_health_reports_imported_cache_entries`
  - `batch_health_reports_remote_manifest_and_pending_age`
- Tests with manipulated ledger/cache fixtures.
- Verify detects stale prompt/source hashes:
  - `batch_verify_accepts_prepared_workspace`
  - `batch_verify_detects_stale_work_item_hashes`
  - `batch_verify_detects_imported_state_without_cache`
- Health output remains stable enough for users to compare runs.

Exit criteria:

- The user can inspect exactly what is done, pending, failed, rejected, and
  stale without opening JSONL files.

## Phase 4: Submit, status, and fetch

Goal: add the OpenAI Batch API network path.

Tasks:

- Implement `batch submit`. Done.
- Upload `requests.jsonl` with `purpose=batch`. Done.
- Create batch with:
  - endpoint `/v1/responses` (implemented)
  - `completion_window: "24h"` (implemented)
- Persist:
  - `file_id` (implemented)
  - `batch_id` (implemented)
  - remote status (implemented)
- Implement `batch status`. Done.
- Implement `batch fetch`. Done.
- Download:
  - `output_file_id` -> `output.jsonl` (implemented)
  - `error_file_id` -> `remote_errors.jsonl` (implemented)
- `batch import` defaults to the fetched `output.jsonl`; `--output <PATH>`
  remains available for fixture or manually downloaded output.
- Make submit/status/fetch safe to rerun. Done for status/fetch; submit refuses
  an existing batch id unless `--force` is set.
- Reuse existing OpenAI API key handling. Done for `OPENAI_API_KEY`,
  `--openai-api-key`, and `--prompt-api-key`.

Verification:

- Small real batch smoke test.
- Re-running status updates manifest without corrupting local state.
- Fetch refuses to overwrite existing output unless `--force` is set.
- Unit coverage:
  - `remote_batch_response_updates_manifest_ids_and_status`
  - `batch_import_defaults_to_fetched_output_file`

Exit criteria:

- A small EPUB can run:
  `prepare -> submit -> status -> fetch -> import -> translate`.

## Phase 5: Retry and local rerouting

Goal: let the user recover unfinished work without manual JSONL editing.

Tasks:

- Implement `batch reroute-local`. Done for marking selected items as
  `local_pending`.
- Implement `batch translate-local`. Done for `local_pending` items through the
  normal provider backend.
- Selection modes:
  - `--state failed` (implemented for arbitrary repeated `--state`)
  - `--state rejected` (implemented)
  - `--remaining` (implemented)
  - `--endgame-threshold N` (implemented)
  - `--limit N` (implemented)
- Mark selected items as `local_pending`. Done.
- Translate `local_pending` through existing backend, usually Ollama. Done.
- Write successful local results to cache immediately. Done.
- Mark successful local results as `local_imported`. Done.
- Preserve provider/model in cache records.

Verification:

- Failed/rejected fixtures can be rerouted and imported locally.
- `--remaining` excludes already imported/cached/skipped items.
- Local translation failures keep state and `last_error`.
- Unit coverage:
  - `batch_reroute_local_marks_selected_state`
  - `batch_reroute_local_respects_endgame_threshold`
  - `batch_reroute_local_short_first_honors_limit`
  - `batch_translate_local_marks_cached_pending_items_imported`

Exit criteria:

- The user can switch remaining batch work to local translation safely.

## Phase 6: Priority scheduling and endgame

Goal: improve speed and reduce tail latency.

Tasks:

- Add priority selection for reroute/retry:
  - `page-order` (implemented)
  - `failed-first` (implemented)
  - `hard-first` (implemented)
  - `short-first` (implemented)
  - `oldest-first` (implemented)
- Define complexity score:
  - source chars (implemented)
  - placeholder count (implemented)
  - previous failure count (partly implemented through attempt/last_error)
- Add explicit endgame flow:
  - show selected remaining count
  - reroute to local or synchronous provider
  - do not make it automatic unless a flag asks for it

Verification:

- Priority ordering unit tests:
  - `batch_reroute_local_short_first_honors_limit`
  - `batch_translate_local_short_first_honors_limit`
- Endgame selection excludes imported/cached items.
- User-facing summaries show why items were selected.

Exit criteria:

- Last few unfinished items can be completed without waiting for another remote
  batch cycle.

## Phase 7: Large-job hardening

Goal: make batch mode safe for large EPUBs and repeated interruptions.

Tasks:

- Split request JSONL before API size/request limits.
- Support multiple batch parts:
  - `batch_part_0001`
  - `batch_part_0002`
- Track per-part remote IDs and statuses.
- Add recovery tests for interrupted:
  - prepare
  - submit
  - fetch
  - import
  - reroute-local
- Add stress tests for concurrent:
  - `translate`
  - `batch import`
  - `batch status`
  - `batch verify`

Verification:

- Multi-part fixture imports correctly.
- Concurrent commands cannot corrupt cache or ledger.
- Interrupted import can be re-run safely.

Exit criteria:

- Batch mode is robust enough for large books and repeated restarts.

## Recommended implementation order

1. Phase 0 lock/cache safety.
2. Phase 1 prepare.
3. Phase 2 fixture import.
4. Phase 3 health/verify.
5. Phase 4 submit/status/fetch.
6. Phase 5 local rerouting.
7. Phase 6 priority/endgame.
8. Phase 7 large-job hardening.

The first useful milestone is the end of Phase 2: even without OpenAI network
calls, epubicus can produce batch requests and import fixture results into the
cache. That proves the split/import/assemble model before remote complexity is
added.
