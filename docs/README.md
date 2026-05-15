# Documentation Index

This directory keeps daily operation guides, detailed runbooks, and design notes separate.

## Start Here

- [../README.ja.md](../README.ja.md) - Japanese quick start, script list, command reference, and recovery flow.
- [operation-guide.ja.md](operation-guide.ja.md) - Daily Japanese operator guide for translation, cache use, recovery, Kindle layout handling, and cleanup.
- [translation-workflow.ja.md](translation-workflow.ja.md) - Step-by-step Japanese workflow from glossary creation to translation and recovery.

## Detailed Runbooks

- [scripts-reference.ja.md](scripts-reference.ja.md) - Per-script reference for `scripts/` (purpose, arguments, usage examples).
- [detailed-examples.ja.md](detailed-examples.ja.md) - Detailed command examples, direct cache/manual recovery, and Send to Kindle notes.
- [runtime-progress.ja.md](runtime-progress.ja.md) - Japanese runtime notes for release-build scripts, ETA measurement, progress display, and inline marker validation.
- [runtime-progress.md](runtime-progress.md) - English runtime/progress notes.
- [batch-recovery.ja.md](batch-recovery.ja.md) - Japanese checklist for recovering unfinished or partially failed OpenAI Batch API runs.
- [batch-recovery.md](batch-recovery.md) - English Batch API recovery checklist.
- [batch-translate-local.ja.md](batch-translate-local.ja.md) - Japanese operator notes for `batch translate-local`, including stop conditions and failure handling.

## Maintenance Notes

- [common-processing.ja.md](common-processing.ja.md) - Map of shared processing paths such as locks, translation, validation, cache, recovery, and batch state handling.
- [cache-recovery-resume-design.ja.md](cache-recovery-resume-design.ja.md) - Design notes for resumable cache, recovery, model switching, and safe partial rebuilds.
- [script-cleanup-plan.ja.md](script-cleanup-plan.ja.md) - Audit of `scripts/` and unification plan for env-template translation scripts.

## Design Notes

- [batch-api-design.md](batch-api-design.md) - Current Batch API design, state model, commands, and recovery model.
- [batch-api-implementation-plan.md](batch-api-implementation-plan.md) - Implementation checklist and completed work notes for Batch API support.
- [multilingual-design.md](multilingual-design.md) - Future multilingual support and target-aware validation design.

## Root Documents

- [../README.md](../README.md) - English quick start and command reference.
- [../CHANGELOG.md](../CHANGELOG.md) - Release history.
