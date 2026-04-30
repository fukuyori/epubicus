# Multilingual translation design

Last updated: 2026-04-30

This document describes how epubicus can evolve from an English-to-Japanese
translator into a multilingual EPUB translation pipeline. The goal is to support
both multilingual input and configurable output languages without making cache,
Batch API, or validation behavior unsafe.

## Goals

- Support configurable source and target languages.
- Allow automatic source-language detection at EPUB block level.
- Avoid calling providers for blocks that are already in the target language
  when the user explicitly enables that optimization.
- Keep mixed-language blocks safe: do not skip a block just because it contains
  some target-language text.
- Make validation target-language aware instead of hard-coding Japanese checks.
- Keep cache and Batch API artifacts isolated by source/target language.
- Update EPUB metadata so the output package advertises the target language.
- Preserve the current English-to-Japanese behavior as the default during the
  transition.

## Non-goals

- Perfect language detection for every short heading, bibliography entry, name,
  or title.
- Automatic translation of only the non-target-language spans inside one XHTML
  block. Initial multilingual support still translates or skips whole extracted
  blocks.
- Claude Batch API support.
- Automatic aggressive skipping by default. Skipping target-language blocks must
  be opt-in until detection quality is proven on real EPUBs.

## Core concepts

### Language settings

Add explicit language settings to the common translation options:

| Option | Environment variable | Default | Meaning |
|--|--|--|--|
| `--source-lang auto|en|ja|zh|ko|fr|de|...` | `EPUBICUS_SOURCE_LANG` | `en` initially | Declared source language, or `auto` for block-level detection |
| `--target-lang ja|en|zh|ko|fr|de|...` | `EPUBICUS_TARGET_LANG` | `ja` | Output language |
| `--skip-target-lang` | `EPUBICUS_SKIP_TARGET_LANG` | false | Skip provider calls for blocks that are confidently already in the target language |

The initial defaults preserve the existing contract: English input translated to
Japanese output.

### Language identifiers

Use BCP 47-like language tags at the CLI and in persisted metadata. Keep the
first implementation focused on common base tags:

- `auto`
- `en`
- `ja`
- `zh`
- `ko`
- `fr`
- `de`
- `es`
- `it`
- `pt`
- `unknown`
- `mixed`

Internally, store detected language as a structured value:

```text
detected_lang = en | ja | zh | ko | latin | unknown | mixed([en, ja])
confidence = 0.00..1.00
reason = "kana ratio high" | "long latin words" | "short text"
```

### Block action

Language detection should not directly mutate output. It should produce an
action recommendation:

| Action | Meaning |
|--|--|
| `translate` | Call provider |
| `skip` | Reuse source text because it is confidently already in target language |
| `warn` | Report ambiguous or mixed-language content but continue according to safe defaults |
| `unknown` | Not enough evidence; treat as `translate` unless user chooses a future stricter mode |

## Language detection strategy

Start with a deterministic, local, character-based detector. This avoids adding
a heavy dependency and keeps behavior explainable.

### Features

For each source block, count:

- Hiragana characters
- Katakana characters
- CJK unified ideographs
- Hangul characters
- Latin letters
- ASCII words
- Digits and punctuation
- Total non-whitespace characters
- Longest Latin text run
- Placeholder count

### Heuristic examples

Japanese:

```text
hiragana + katakana >= 4
and japanese chars / non-space chars is high enough
```

English / Latin-script language:

```text
latin letters >= 12
and ascii words >= 3
and target-specific non-Latin chars are low
```

Chinese:

```text
CJK characters are dominant
and hiragana + katakana is near zero
```

Korean:

```text
Hangul characters are present above a small threshold
```

Mixed:

```text
two language families each exceed a meaningful threshold
or target language is present but a long source-language run remains
```

Unknown:

```text
short headings, names, numeric-only lines, punctuation-only lines,
or bibliography lines where no language dominates
```

## Skip policy

`--skip-target-lang` should be conservative.

For `target-lang=ja`, skip only when all are true:

- The block has enough Japanese characters.
- Hiragana or katakana is present, or the block is otherwise very confidently
  Japanese.
- There is no long Latin run.
- ASCII word ratio is low.
- The block is not marked mixed.
- The block is not too short to classify safely unless it is clearly Japanese.

Examples:

| Source block | Target | Action | Reason |
|--|--|--|--|
| `これはすでに日本語の本文です。` | `ja` | skip | already target language |
| `これは New York University の説明です。` | `ja` | skip | Japanese dominant, short proper noun |
| `序論` | `ja` | skip | short but clearly Japanese |
| `INTRODUCTORY` | `ja` | translate | source language likely remains |
| `これは日本語です。The bibliography will suggest...` | `ja` | translate | long Latin run remains |
| `ABRAM S. ISAACS` | `ja` | translate or unknown | short proper name; do not skip by default |

For non-Japanese targets, use equivalent target-language confidence rules. If a
language is not yet supported by a target-specific detector, do not enable
automatic skip for that target.

## Prompt design

Replace hard-coded English-to-Japanese prompt text with language-aware prompt
building.

Prompt inputs:

- source language setting
- detected language, when available
- target language
- style preset
- glossary subset
- placeholder rules

Example high-level system prompt:

```text
You are a professional book translator.
Translate the source text into {target_language_name}.
The source language is {source_language_description}.
Preserve every placeholder marker exactly.
Return only the translation.
```

For `target-lang=ja`, existing Japanese style rules can remain. For other
target languages, add a neutral default style first, then expand style presets
later.

Retry prompts must also use target-language-aware wording. The validation error
can remain diagnostic, but the instruction should not say "Japanese" unless the
target is Japanese.

## Validation design

Current validation checks are Japanese-centric. Replace them with a target-aware
validator:

```text
validate_translation_response(source, translated, language_policy)
```

Validation policy should include:

- source language setting
- target language
- detected source language
- whether mixed output is allowed
- maximum tolerated source-language residue
- placeholder signature

### Shared validation

Always reject:

- empty output
- prompt wrapper leakage
- missing or changed inline placeholders
- unchanged source when the source is meaningful and not already target language
- refusal/explanation responses
- suspicious truncation

### Target-specific validation

For `target-lang=ja`:

- Require Japanese characters unless the source is a citation/name line that is
  allowed to remain mostly Latin.
- Reject long untranslated Latin runs when the source contains meaningful Latin
  text.

For `target-lang=en`:

- Require meaningful Latin text unless the source is a citation/name line.
- Reject large remaining Japanese/CJK/Hangul runs when the source is not already
  English.

For `target-lang=ko`:

- Require Hangul for normal prose.
- Reject large remaining non-target-language runs.

For `target-lang=zh`:

- Require CJK text, but distinguish Japanese by kana presence where possible.
- Treat Japanese/Chinese ambiguity carefully and prefer warning over false
  rejection for short text.

## Cache design

Language settings must be part of the cache key. Otherwise an English-to-Japanese
translation could be reused accidentally for Japanese-to-English or
English-to-Chinese.

Add to cache key inputs:

- source language setting
- target language
- prompt version
- style
- glossary subset
- provider
- model
- source text

Do not include low-confidence detected language in the key for the first
implementation; it can change as heuristics improve and would invalidate caches
too aggressively. Instead, store detected language in metadata/reporting.

Cache manifest additions:

```json
{
  "source_lang": "en",
  "target_lang": "ja",
  "language_detection_version": "v1"
}
```

Cache record optional additions:

```json
{
  "detected_lang": "mixed(en,ja)",
  "language_confidence": 0.82,
  "language_action": "translate"
}
```

## Batch API design impact

Batch mode must persist the language context used at prepare time.

`work_items.jsonl` additions:

```json
{
  "source_lang": "auto",
  "target_lang": "ja",
  "detected_lang": "en",
  "language_confidence": 0.91,
  "language_action": "translate",
  "language_reason": "latin words dominate"
}
```

`batch verify` should report stale items when:

- source language setting changed
- target language changed
- prompt language policy changed
- glossary language does not match

`batch prepare --skip-target-lang` should mark confidently skipped work as
`skipped` or omit it from remote requests while keeping enough ledger state for
`batch health` and `batch verify` to explain why it was skipped.

## EPUB metadata

Output EPUB metadata should reflect the target language.

Update:

- OPF `dc:language`
- XHTML `lang`
- XHTML `xml:lang`

When feasible, preserve source-language history in metadata:

```xml
<meta property="epubicus:source-language">en</meta>
<meta property="epubicus:target-language">ja</meta>
```

This should be done carefully because adding undeclared namespaces can break XML.
Prefer existing EPUB-compatible metadata patterns already used in the project.

## Glossary impact

Glossary JSON already has `source_lang` and `target_lang` fields. Multilingual
support should start enforcing them.

Behavior:

- If glossary has no language metadata, accept with a warning.
- If glossary `source_lang` conflicts with explicit `--source-lang`, warn or
  reject depending on strictness.
- If glossary `target_lang` conflicts with `--target-lang`, reject by default.
- Glossary review prompts should be generated for the configured target
  language.

## CLI proposal

```powershell
cargo run -- translate .\book.epub --source-lang auto --target-lang ja
cargo run -- translate .\book.epub --source-lang en --target-lang zh
cargo run -- translate .\book.epub --source-lang auto --target-lang ja --skip-target-lang
cargo run -- language-report .\book.epub --source-lang auto --target-lang ja
```

`test`, `translate`, `batch prepare`, `batch run`, and `glossary` should share
the same language options where relevant.

## Language report command

Add a read-only command before enabling behavior-changing skip logic.

```powershell
cargo run -- language-report .\book.epub --from 3 --to 5 --target-lang ja
```

Example output:

```text
Language report
input: book.epub
source_lang: auto
target_lang: ja
pages: 3..5

page 3 block 12 | en | translate | confidence 0.93 | latin words dominate
page 3 block 13 | ja | skip-candidate | confidence 0.96 | already target language
page 3 block 14 | mixed(en,ja) | translate | confidence 0.82 | long latin run remains
page 4 block 2  | unknown | translate | confidence 0.20 | short text

summary: translate 88 | skip-candidate 12 | unknown 7 | mixed 3
```

The first implementation should not mutate cache or output.

## Development schedule

The schedule below is organized as implementation phases rather than calendar
dates. Each phase should be independently testable and documented before moving
on.

### Phase 1: Language option foundation

Goal: introduce language settings without changing existing behavior.

Tasks:

- Add `LanguageTag` or equivalent parser.
- Add `--source-lang` and `--target-lang` to common args.
- Add `EPUBICUS_SOURCE_LANG` and `EPUBICUS_TARGET_LANG`.
- Keep defaults `en` and `ja`.
- Include language settings in cache key.
- Add language settings to cache manifest.
- Update README and templates.

Verification:

- Existing English-to-Japanese commands still work without new flags.
- Cache keys differ between `--target-lang ja` and `--target-lang en`.
- Help output lists the new flags.

Exit criteria:

- Language settings are persisted and cache-safe, but no behavior-changing
  detection is active yet.

### Phase 2: Prompt multilingualization

Goal: allow target-language-specific provider prompts.

Tasks:

- Replace hard-coded English-to-Japanese system prompt with a language-aware
  prompt builder.
- Add target-language display names.
- Keep Japanese style prompts for `target-lang=ja`.
- Add neutral generic style prompts for other target languages.
- Update retry prompt wording.
- Update tests that assert Japanese-only validation messages.

Verification:

- Prompt snapshots or unit tests for `en -> ja`, `ja -> en`, and `auto -> ja`.
- Existing Japanese output path remains unchanged enough to avoid a cache
  surprise beyond the planned prompt-version bump.

Exit criteria:

- The provider can be instructed to translate into a non-Japanese target.

### Phase 3: Read-only language detection and report

Goal: make detection observable before it changes translation behavior.

Tasks:

- Implement deterministic character-feature extraction.
- Implement block-level language classification.
- Add `language-report` command.
- Add summary counts by detected language and recommended action.
- Add JSON output option if useful for later automation.

Verification:

- Unit tests for Japanese, English, mixed, Korean, Chinese-like, short unknown,
  bibliography-like, and proper-name lines.
- Run report against sample EPUBs and inspect false positives.

Exit criteria:

- Users can see what would be skipped or translated, but translation behavior is
  unchanged.

### Phase 4: Conservative target-language skip

Goal: reduce cost/time for already-translated blocks while avoiding missed
translations.

Tasks:

- Add `--skip-target-lang`.
- Add skip decision only for high-confidence target-language blocks.
- Do not skip `mixed` or `unknown`.
- Record skip counts in progress and final summaries.
- Make `--usage-only` account for skipped blocks.
- Add cache/report metadata for skipped decisions without provider calls.

Verification:

- Japanese-only blocks are skipped for `target-lang=ja`.
- Blocks with long English residue are not skipped.
- Proper names and bibliography entries do not cause broad false skips.
- A run without `--skip-target-lang` behaves as before.

Exit criteria:

- Skipping is opt-in, explainable, and conservative.

### Phase 5: Target-aware validation

Goal: remove Japanese-specific assumptions from validation.

Tasks:

- Replace `validate_translation_response(source, translated)` with a
  language-policy-aware variant.
- Keep shared validation for placeholders, wrappers, unchanged source, refusal,
  and truncation.
- Add target-specific residue checks for Japanese, English, Korean, and basic
  Chinese.
- Update cached-translation validation.
- Update retry error messages to name the target language.

Verification:

- Existing Japanese validation tests still pass.
- English target tests reject large Japanese residue.
- Mixed accepted cases remain accepted when only short proper nouns remain.
- Batch import uses the same target-aware validation.

Exit criteria:

- Validation works for at least Japanese and English targets without relying on
  hard-coded Japanese-only checks.

### Phase 6: Batch and work ledger integration

Goal: make asynchronous workflows language-safe.

Tasks:

- Store source/target language and detected language in work items.
- Include language policy hash in prompt hash.
- Make `batch verify` detect language mismatch as stale.
- Make `batch prepare --skip-target-lang` represent skipped items safely.
- Update `batch health` summary with skipped/language counts.
- Ensure `batch retry-requests`, `reroute-local`, and `translate-local` preserve
  language policy.

Verification:

- Preparing the same EPUB with different target languages creates distinct
  request/cache keys.
- Import rejects output produced under mismatched language policy.
- Verify reports stale items after target language change.

Exit criteria:

- Batch mode cannot mix translations across language directions.

### Phase 7: EPUB metadata output

Goal: make produced EPUBs advertise the configured target language.

Tasks:

- Update OPF `dc:language`.
- Update XHTML `lang` and `xml:lang`.
- Add safe metadata for source/target language if appropriate.
- Ensure no undeclared namespace is introduced.

Verification:

- EPUB XML remains parseable.
- Output language metadata changes with `--target-lang`.
- Existing Japanese output still uses `ja`.

Exit criteria:

- Readers and downstream tools can identify the output language.

### Phase 8: Glossary multilingualization

Goal: make glossary behavior consistent with language settings.

Tasks:

- Validate glossary `source_lang` and `target_lang`.
- Warn on missing glossary language metadata.
- Reject target-language mismatch by default.
- Update glossary candidate and review prompt generation for target language.
- Add docs and examples.

Verification:

- `target_lang=ja` glossary continues to work.
- `target_lang=en` rejects a `target_lang=ja` glossary unless an explicit
  future override is added.
- Review prompt asks for terms in the configured target language.

Exit criteria:

- Glossary data cannot silently contaminate another language direction.

### Phase 9: Documentation and templates

Goal: make multilingual usage discoverable.

Tasks:

- Update README quick start and option tables.
- Update `docs/operation-guide.ja.md`.
- Add examples for:
  - English to Japanese
  - Japanese to English
  - auto source detection
  - skip already-target-language blocks
  - Batch mode with language flags
- Update PowerShell and POSIX templates.

Verification:

- Template commands run with the new flags.
- README examples match CLI help.

Exit criteria:

- Users can run multilingual workflows without reading implementation notes.

## Risk register

| Risk | Impact | Mitigation |
|--|--|--|
| False skip of untranslated content | Output leaves source text untranslated | Keep skip opt-in and conservative; report decisions |
| False rejection of valid translation | Extra retries or failed runs | Target-specific allowlists for short names/citations |
| Cache contamination across directions | Wrong-language output reused | Include source/target language in cache key |
| Batch import from old prompt/language | Wrong output enters cache | Store language policy hash and verify as stale |
| Japanese/Chinese ambiguity | Misclassification | Treat short CJK-only text as unknown unless confident |
| Bibliography-heavy pages | Bad skip/validation decisions | Add bibliography-like tests and report categories |
| Prompt drift changes cache unexpectedly | Cache misses increase | Explicit prompt version bump and documentation |

## Recommended first milestone

The first practical milestone should include Phases 1-3 only:

1. `--source-lang` / `--target-lang`
2. language-aware prompts
3. read-only `language-report`

This gives users visibility into detection quality before the pipeline starts
skipping provider calls. Phases 4-6 should follow only after the report output
looks reliable on real EPUB samples.
