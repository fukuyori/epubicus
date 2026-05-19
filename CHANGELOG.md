# Changelog

All notable changes to epubicus are documented in this file.

## 0.4.5 - 2026-05-16

### Changed

- Untranslated blocks remaining after rebuild are now reported as `warning:` with exit code 0 (previously `error:` with exit code 2). The EPUB is produced either way; only true fatal errors (cannot write EPUB) keep a non-zero exit code.
- Reorganized `scripts/`: renamed `*-env.template.{ps1,sh}` to `translate-<provider>.{ps1,sh}` (dropped `local-` prefix for ollama).
- Unified the cache root to `<ProjectRoot>/.cache/` for all scripts. Provider is included in `cache_key`, so multiple providers can coexist safely.

### Added

- New generic scripts: `usage.{ps1,sh}` and `rebuild-from-cache.{ps1,sh}` (with provider/model auto-detection from cache).
- Shell counterparts for `inspect-epub` / `create-glossary` / `recover-from-cache` / `scan-and-recover` / `batch-recover-local` / `clear-all-caches`.
- `-h` / `--help` flag on every script (prints the leading comment block).
- Short option aliases for common parameters: `-p` (Provider), `-m` (Model), `-f` (From), `-t` (To), `-c` (Concurrency), `-g` (Glossary), `-l` (Limit), `-mn` (Manual), `-s` (Style), `-cr` (CacheRoot).
- `docs/scripts-reference.ja.md` (per-script reference) and `docs/script-cleanup-plan.ja.md` (audit and migration notes).

### Removed

- Provider-specific thin wrappers that duplicated body scripts: `convert-deepseek` / `page-deepseek` / `usage-deepseek` / `recover-deepseek` / `recover-openai` / `manual-recover-deepseek` / `scan-deepseek` / `scan-recover-deepseek` / `rebuild-deepseek`. Use the unified bodies (`translate-<provider>`, `usage`, `recover-from-cache`, `scan-and-recover`, `rebuild-from-cache`) directly.

### Fixed

- `scan-and-recover` now requires `-Provider` explicitly (no implicit ollama default), aligning with `usage` for safer invocation in multi-provider environments.
- `scan-and-recover.{ps1,sh}` now always runs a `rebuild-from-cache` fallback after the inner scan-recovery step so the EPUB always reflects successful cache updates, even when some blocks remain unrecoverable (the inner rebuild is skipped in that case by design). The fallback is skipped only when `-ScanOnly` / `-NoRebuild` / `-NoRun` is given.

## 0.4.4 - 2026-05-15

### Fixed

- XHTML parser no longer rejects EPUBs that use HTML5 void-element syntax (`<img>`, `<br>`, `<hr>`, etc. without a self-closing slash). Source bytes are normalized to self-closing form before parsing.
- Stale-lock detection now works on non-English Windows locales. The `tasklist` fallback no longer relies on the English `INFO:` prefix and uses the CSV double-quote prefix instead.

## 0.4.3 - 2026-05-08

### Added

- Added EPUB metadata-based source language detection for glossary generation and translation prompts.
- Added multilingual source support while keeping the translation target fixed to Japanese.
- Added a `-DevBuild` option to `create-glossary.ps1` for debug-build glossary checks.

### Changed

- Translation prompts now use the EPUB source language hint when available and fall back to automatic source-language detection.
- Glossary output now records the detected source language instead of assuming English.
- Validation now treats unchanged non-Japanese source text as untranslated across multilingual inputs.
- Scan recovery now hides per-block suspicious-output messages unless `--verbose` is used.

## 0.4.2 - 2026-05-06

### Added

- Added DeepSeek helper scripts for inspection, usage checks, trial translation, full conversion, cache rebuild, recovery, scan recovery, and manual cache recovery.
- Added DeepSeek API provider support through the Anthropic-compatible messages API.
- Added manual recovery JSON support so selected recovery items can be written directly into the translation cache without another provider call.
- Added a Mermaid recovery flow diagram to the Japanese README.
- Added documentation for script-first DeepSeek workflows, novel style options, direct cache/manual recovery, and recovery reason filters.

### Changed

- `convert-deepseek.ps1` now passes through extra `translate` options such as `--style novel` and `--style novel-polite`.
- Recovery progress now reports updated, failed, cached, and retry counts without streaming detailed per-item messages unless `--verbose` is used.
- Translation progress now hides internal XHTML filenames and shows recoverable-output counts as a running total.
- Request retry and validation retry are now controlled separately with `--retries` and `--validation-retries`.
- Recovery now removes successfully handled records from `recovery.jsonl`, so interrupted recovery runs keep completed work.
- Recovery summaries now include per-reason counts for updated, already-valid, and unrecoverable items.

### Fixed

- Placeholder-preserving recovery now falls back to a normal validation retry if segment-level recovery cannot translate any text.
- Safe structural passthrough handling now avoids repeated provider retries for hard reference-like content such as URLs, email addresses, identifiers, code-like snippets, and reference/index entries.
- Cache replacement during recovery now updates existing invalid entries instead of leaving stale invalid translations in place.
- Kindle fixed-layout metadata can now be applied automatically for fixed-layout-like EPUBs, with script controls for `auto`, `fixed`, and `reflow`.

## 0.4.1 - 2026-05-04

### Added

- Added helper scripts for glossary creation, cache cleanup, cache-based recovery, batch local recovery, and scan-based recovery.
- Added a Japanese translation workflow guide covering glossary creation, conversion methods, and recovery flows.

### Changed

- Translation helper scripts now auto-use a same-directory, same-basename glossary JSON when one exists beside the input EPUB.
- OpenAI Batch helper defaults now use a 180-second polling interval.
- Batch local recovery now continues after verification warnings by default, with `-StrictVerify` available when a hard stop is preferred.

### Fixed

- Fixed fixed-layout EPUB popup text extraction so popup `div` blocks such as `id="popup-..."` are included in inspection, batch preparation, recovery scan, and translation work.

## 0.4.0 - 2026-05-04

### Added

- Added `docs/batch-translate-local.ja.md` for the `batch translate-local` flow, including progress display, stop conditions, `last_error`, and recovery choices.
- Added `docs/common-processing.ja.md` to map shared processing paths such as locks, cache, validation, recovery records, batch state transitions, and progress handling.

### Changed

- `batch translate-local` now saves item state as it progresses, shows completed/error counts in progress output, and records fuller provider error details in `last_error`.
- Local batch retry now separates reference-like untranslated blocks from prose-like blocks, so reference-style content is quickly moved out of the local retry lane instead of consuming repeated paid retries.
- Reference passthrough cache entries are now treated as intentional original preservation during `--partial-from-cache` assembly and batch verification.
- Removed the repo-local Cargo target-dir override so normal `cargo build --release` updates `target/release`.
- OpenAI Batch `run --wait` now reports poll count, elapsed time, part status counts, remote completed requests, failed requests, and the next poll interval.
- OpenAI Batch submit now saves the manifest after each uploaded/submitted part, reducing duplicate uploads and remote submissions if a multi-part submit is interrupted.
- `batch run` now prints total elapsed time when it completes or pauses before fetchable remote output is ready.
- `translate`, `test`, and usage-estimate runs now print total elapsed time when they finish.
- Translation and OpenAI Batch runs now persist cumulative active elapsed time in their manifests, so resumed runs can report total active time across interruptions.

### Fixed

- Recovered stale input `run.lock` and batch lock files more reliably after interrupted runs.
- Stopped local batch processing immediately on provider authentication failures and on long stalls where requests increase without new completed items.
- Prevented intentionally preserved reference blocks from being emitted again as untranslated recovery records during final EPUB assembly.

## 0.3.9 - 2026-05-03

### Added

- Added runtime/progress notes in English and Japanese, covering release-build helper scripts, ETA measurement, and inline marker validation.

### Changed

- Simplified ETA calculation so resumed runs measure only the uncached source characters counted at startup, using the current run's provider elapsed time and completed uncached characters.
- Excluded spine pages 1-3 from ETA timing and character totals, and kept ETA hidden as `ETA pending` until page 4 or later has at least five minutes of provider work measured.
- Switched helper script templates to `cargo run --release -- ...` so normal scripted conversions use release builds.

### Fixed

- Rejected provider output that adds bracket-style inline markers such as `⟦/S1⟧` or `⟦DAX⟧`, preventing unresolved tag-restoration markers from reaching the EPUB output.

## 0.3.7 - 2026-05-02

### Added

- Added a documentation index under `docs/` so operator guides, recovery notes, and design documents are easier to find.

### Changed

- ETA now measures from the current run or resume point using the uncached source characters counted at startup, without carrying cached work or later baseline adjustments into the estimate.
- Validation failures now carry machine-readable reasons, and retry prompts use those reasons to give targeted, generic English correction instructions.

### Fixed

- Avoided double-counting validation passthrough blocks in progress and labelled them as `validation_passthrough`.

## 0.3.6 - 2026-05-01

### Added

- Added recovery logging for untranslated or original-output blocks under the cache directory (`recovery/<output-name>/recovery.jsonl` and `untranslated.txt`).
- Added `recover` to retry selected recovery-log items, write unrecoverable items to `failed.jsonl`, and optionally rebuild the EPUB from cache.
- Added `scan-recovery` to compare an output EPUB against the original and create recovery logs for suspicious untranslated blocks.
- Added recovery-log counts and paths to `cache list` and `cache show`, including `recover`-ready log paths.
- Added `--verbose` / `EPUBICUS_VERBOSE` so retry, fallback, concurrency, and long-block warnings are opt-in.
### Changed

- `--partial-from-cache` now reports recoverable failures when untranslated blocks remain after writing the EPUB and recovery artifacts.
- Recovery and untranslated artifacts are stored with the cache, so cache cleanup commands can manage them together.
- Glossary candidate output now focuses on `src` / `dst`, while existing `kind` and `note` fields remain readable but are not sent to providers.
- Glossary cache keys now use trimmed `src` / `dst` pairs and ignore empty translations.
- Detailed batch recovery documentation was expanded in English and Japanese.

### Fixed

- Kept invalid cached translations and validation warnings quiet unless verbose output is requested.
- Preserved original XHTML blocks and logged recovery records when inline placeholder restoration is unsafe.
- Kept verification build artifacts out of the project root by moving existing `target-*` directories into `target-runs`.
