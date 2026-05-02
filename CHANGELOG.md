# Changelog

All notable changes to epubicus are documented in this file.

## 0.3.8 - 2026-05-03

### Changed

- Simplified ETA calculation so resumed runs measure only the uncached source characters counted at startup, using the current run's provider elapsed time and completed uncached characters.

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
- Added repo-local Cargo target configuration so default build and verification artifacts go under `target-runs/default`.

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
