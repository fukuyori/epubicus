# Runtime and Progress Notes

This note records operational behavior that matters during long translation
runs.

## Release Builds

The helper script templates under `scripts/` run epubicus through release
builds:

```powershell
cargo run --release -- ...
```

Use the scripts for normal long-running conversions. The root README keeps
plain `cargo run -- ...` examples for development-oriented command reference
and quick smoke checks.

## ETA Measurement

ETA is intentionally delayed and scoped to body-like work.

- Spine pages 1-3 are excluded from ETA timing and character totals.
- ETA timing starts when provider translation starts on spine page 4 or later.
- ETA remains `ETA pending` until that page-4-or-later provider timing has run
  for at least five minutes.
- The progress bar position still includes all selected blocks, including spine
  pages 1-3 and cached blocks.
- If the selected range contains only spine pages 1-3, ETA is not measured and
  remains pending until completion.

After ETA starts, the estimate uses the current run or resume point only:
completed uncached source characters from spine page 4 onward divided by
provider elapsed time, projected over the remaining uncached source characters
from spine page 4 onward.

## Inline Marker Validation

epubicus replaces inline XHTML with temporary markers before translation:

- `⟦E1⟧` and `⟦/E1⟧` wrap paired inline tags.
- `⟦S1⟧` represents a self-closing inline tag.

Provider responses are rejected before cache writes if they drop, change, or add
bracket-style markers. This includes invalid generated markers such as
`⟦/S1⟧` and decorative markers such as `⟦DAX⟧`. If an invalid cached or imported
response still reaches EPUB assembly, unresolved marker text causes inline
restore failure and the original XHTML block is preserved for recovery instead
of leaking the marker into the output EPUB.
