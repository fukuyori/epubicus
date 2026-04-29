# epubicus

`epubicus` is a CLI tool for translating English EPUB files into Japanese while keeping the EPUB package structure and XHTML formatting intact.

It currently supports local Ollama, OpenAI, and Claude providers.

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

Translation results are cached per-input EPUB under an OS-standard cache root (Windows: `%LOCALAPPDATA%\epubicus\cache`, Unix: `~/.cache/epubicus`). Each input gets its own subdirectory named after the SHA-256 hash of its bytes, with `manifest.json` and `translations.jsonl` inside.

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --cache-root .\.epubicus-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --clear-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --no-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --keep-cache
```

After an interrupted run, rerun the same `translate` command to resume from uncached blocks. Because the cache directory is keyed by input EPUB hash, resuming works regardless of the output path. The progress bar starts at the cached position and shows a message such as `resuming: 991/5805 cached`.

On a successful full-range translation, the cache directory is **automatically deleted**. Pass `--keep-cache` to retain it (useful for debugging or to keep entries available for partial reuse).

Create a partial translated EPUB from cache only, leaving cache misses unchanged. This mode is **read-only on the cache** (no manifest update, no auto-delete):

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --partial-from-cache
```

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
```

`translate` creates an EPUB and shows a progress bar with elapsed time, ETA, selected spine pages, and translatable XHTML block count. ETA stays as `ETA warming up` until at least 5 uncached model-translated blocks and 30 seconds have been observed.

`test` prints translated text for a selected spine range to stdout. It does not create an EPUB.

`inspect` shows OPF path, spine order, `linear` state, referenced file existence, file size, and a rough count of translatable XHTML blocks.

`toc` shows EPUB3 `nav.xhtml` or EPUB2 NCX table-of-contents entries with indentation and target hrefs.

`glossary` extracts candidate proper nouns and terms into JSON for manual review.

## Options

### `translate`

| Option | Default | Description |
|--|--|--|
| `-o, --output PATH` | `<input>.ja.epub` | Output EPUB |
| `--from N` | first spine item | First 1-based OPF spine number to translate |
| `--to N` | last spine item | Last 1-based OPF spine number to translate |
| `--partial-from-cache` | false | Replace cache hits with translations and keep cache misses unchanged |

### `test`

| Option | Default | Description |
|--|--|--|
| `--from N` | required | First 1-based OPF spine number to print |
| `--to N` | required | Last 1-based OPF spine number to print |

### Shared `translate` / `test` Options

| Option | Default | Description |
|--|--|--|
| `--provider ollama\|openai\|claude` | `ollama` | Translation provider |
| `-m, --model NAME` | provider-specific | Model name |
| `--ollama-host URL` | `http://localhost:11434` | Ollama endpoint |
| `--openai-base-url URL` | `https://api.openai.com/v1` | OpenAI API base URL |
| `--claude-base-url URL` | `https://api.anthropic.com/v1` | Claude / Anthropic API base URL |
| `--openai-api-key KEY` | none | OpenAI API key |
| `--anthropic-api-key KEY` | none | Anthropic API key |
| `--prompt-api-key` | false | Prompt for the provider API key without echoing it |
| `--temperature F` | `0.3` | Sampling temperature |
| `--num-ctx N` | `8192` | Context window size passed to Ollama |
| `--timeout-secs N` | `900` | HTTP timeout per request, in seconds |
| `--retries N` | `2` | Retries for timeouts, connection errors, rate limits, and server errors |
| `--style STYLE` | `essay` | Style preset: `novel`, `novel-polite`, `tech`, `essay`, `academic`, `business` |
| `--dry-run` | false | Do not call a provider; use source text to smoke-test EPUB handling |
| `--glossary PATH` | none | Glossary JSON for consistent terms |
| `--cache-root PATH` | OS cache (`%LOCALAPPDATA%\epubicus\cache` / `~/.cache/epubicus`) | Override the cache root. Per-EPUB caches live under `<cache-root>/<input-hash>/` |
| `--no-cache` | false | Do not read or write the cache. Existing cache files are not deleted |
| `--clear-cache` | false | Delete this input EPUB's cache before translating |
| `--keep-cache` | false | Keep the cache after a successful completion (default: cache is auto-deleted) |

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
| `cache list` | List all cached runs with hash, segment count, size, last update, and input path |
| `cache show <hash\|input.epub>` | Print the manifest for one run (resolved by hash prefix or input EPUB path) |
| `cache prune --older-than <DAYS> [--yes] [--dry-run]` | Delete runs whose `last_updated_at` is older than N days |
| `cache clear --hash <HASH> [--dry-run]` | Delete one cached run |
| `cache clear --all [--yes] [--dry-run]` | Delete every cached run. Requires typing `yes` unless `--yes` is set |

`cache` accepts `--cache-root <PATH>` to operate on a non-default cache root.

## Providers

Ollama is the default provider and runs locally:

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

Send `glossary-review.md` to ChatGPT or Claude, then save the returned JSON as `glossary.json` and use it for translation. The prompt asks the model to remove false positives, merge duplicates, classify entries such as `person`, `place`, `organization`, or `term`, and fill Japanese `dst` suggestions.

`glossary-review.md` is self-contained: it includes explanatory comments, field meanings, review rules, and the candidate JSON, so it can be pasted directly into ChatGPT or Claude. `glossary.candidates.json` remains valid comment-free JSON.

Edit `dst` values:

```json
{
  "source_lang": "en",
  "target_lang": "ja",
  "entries": [
    {
      "src": "Horizon",
      "dst": "ホライゾン",
      "kind": "term",
      "note": "occurrences: 793"
    }
  ]
}
```

Use the glossary during translation:

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --glossary .\glossary.json
```

Only entries whose `src` appears in the current block are sent to the provider, so the prompt does not include the entire glossary every time.

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

## Limitations

- EPUB reader page numbers are not used. Ranges are OPF spine numbers.
- `--partial-from-cache` does not call a model, replaces cache hits with translated text, and leaves cache misses unchanged. It cannot be combined with `--no-cache`.
- `nav.xhtml` / NCX display is implemented, but TOC translation is not implemented yet.
- Retry policy and detailed fallback reports are not implemented yet.
- Code/preformatted content is protected from translation.
- Provider pricing estimates are not implemented.

## Troubleshooting

If `failed to open .\book.epub` appears, the file does not exist at that path. `book.epub` is only an example name.

```powershell
Get-ChildItem -Filter *.epub
cargo run -- inspect .\actual-file-name.epub
```

If `ollama` is not found, either add Ollama to PATH or use the full executable path shown above.
