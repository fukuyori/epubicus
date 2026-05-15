#!/usr/bin/env sh
# Recover untranslated blocks from the newest recovery log for an input EPUB.
# Provider-agnostic body for recovery operations.
#
# Usage:
#   chmod +x scripts/recover-from-cache.sh
#   scripts/recover-from-cache.sh ./book.epub --provider deepseek
#   scripts/recover-from-cache.sh ./book.epub --provider openai
#   scripts/recover-from-cache.sh ./book.epub --provider deepseek --manual ./book.manual.json
#   scripts/recover-from-cache.sh ./book.epub --provider deepseek --list
#   scripts/recover-from-cache.sh ./book.epub --provider deepseek --no-rebuild

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
PROVIDER="ollama"
MODEL=""
CONCURRENCY="0"
CACHE_ROOT=""
GLOSSARY=""
MANUAL=""
LIMIT="0"
PAGE="0"
BLOCK="0"
REASONS=""
OUTPUT=""
NO_REBUILD="0"
KINDLE_FIXED="0"
NO_KINDLE_FIXED="0"
LIST="0"
DEV_BUILD="0"
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --provider|-p) PROVIDER="$2"; shift 2 ;;
        --model|-m) MODEL="$2"; shift 2 ;;
        --concurrency|-c) CONCURRENCY="$2"; shift 2 ;;
        --cache-root|-cr) CACHE_ROOT="$2"; shift 2 ;;
        --glossary|-g) GLOSSARY="$2"; shift 2 ;;
        --manual|-mn) MANUAL="$2"; shift 2 ;;
        --limit|-l) LIMIT="$2"; shift 2 ;;
        --page) PAGE="$2"; shift 2 ;;
        --block) BLOCK="$2"; shift 2 ;;
        --reason) REASONS="$REASONS
$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --no-rebuild) NO_REBUILD="1"; shift ;;
        --kindle-fixed-layout) KINDLE_FIXED="1"; shift ;;
        --no-kindle-fixed-layout) NO_KINDLE_FIXED="1"; shift ;;
        --list) LIST="1"; shift ;;
        --dev-build) DEV_BUILD="1"; shift ;;
        --no-run) NO_RUN="1"; shift ;;
        --) shift; break ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *)
            if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$1"; shift
            else echo "unexpected argument: $1" >&2; exit 2; fi
            ;;
    esac
done

if [ "$KINDLE_FIXED" = "1" ] && [ "$NO_KINDLE_FIXED" = "1" ]; then
    echo "error: --kindle-fixed-layout and --no-kindle-fixed-layout cannot be used together" >&2
    exit 2
fi

case "$PROVIDER" in
    ollama|openai|claude|deepseek) ;;
    *) echo "error: invalid --provider: $PROVIDER" >&2; exit 2 ;;
esac

if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$PROJECT_ROOT/test/sample.epub"; fi
INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_EPUB="$INPUT_DIR/$INPUT_FILE"

# Provider-specific Model defaults (only used when --model is omitted).
if [ -z "$MODEL" ]; then
    case "$PROVIDER" in
        ollama)   MODEL="qwen3:14b" ;;
        openai)   MODEL="gpt-5-mini" ;;
        claude)   MODEL="claude-sonnet-4-5" ;;
        deepseek) MODEL="deepseek-v4-flash" ;;
    esac
fi

if [ -z "$CACHE_ROOT" ]; then CACHE_ROOT="$PROJECT_ROOT/.cache"; fi

# Glossary auto-detect when not specified.
GLOSSARY_PATH=""
INPUT_BASE=${INPUT_FILE%.*}
if [ -n "$GLOSSARY" ]; then
    GLOSSARY_PATH="$GLOSSARY"
elif [ -f "$INPUT_DIR/$INPUT_BASE.json" ]; then
    GLOSSARY_PATH="$INPUT_DIR/$INPUT_BASE.json"
fi

set -- recover \
    --cache "$INPUT_EPUB" \
    --provider "$PROVIDER" \
    --model "$MODEL" \
    --cache-root "$CACHE_ROOT"

if [ "$CONCURRENCY" -gt 0 ] 2>/dev/null; then set -- "$@" --concurrency "$CONCURRENCY"; fi
if [ "$LIMIT" -gt 0 ] 2>/dev/null; then set -- "$@" --limit "$LIMIT"; fi
if [ -n "$GLOSSARY_PATH" ]; then set -- "$@" --glossary "$GLOSSARY_PATH"; fi
if [ -n "$MANUAL" ]; then set -- "$@" --manual "$MANUAL"; fi
if [ "$PAGE" -gt 0 ] 2>/dev/null; then set -- "$@" --page "$PAGE"; fi
if [ "$BLOCK" -gt 0 ] 2>/dev/null; then set -- "$@" --block "$BLOCK"; fi

OLD_IFS=$IFS; IFS='
'
for r in $REASONS; do
    [ -z "$r" ] && continue
    OLD_IFS_INNER=$IFS; IFS=$OLD_IFS
    set -- "$@" --reason "$r"
    IFS=$OLD_IFS_INNER
done
IFS=$OLD_IFS

if [ "$LIST" = "1" ]; then
    set -- "$@" --list
elif [ "$NO_REBUILD" = "0" ]; then
    set -- "$@" --rebuild
fi

if [ -n "$OUTPUT" ]; then set -- "$@" --output "$OUTPUT"; fi
if [ "$KINDLE_FIXED" = "1" ]; then set -- "$@" --kindle-fixed-layout; fi
if [ "$NO_KINDLE_FIXED" = "1" ]; then set -- "$@" --no-kindle-fixed-layout; fi

echo
echo "InputEpub  = $INPUT_EPUB"
if [ "$LIST" = "0" ] && [ "$NO_REBUILD" = "0" ]; then
    if [ -n "$OUTPUT" ]; then echo "OutputEpub = $OUTPUT"
    else echo "OutputEpub = $INPUT_DIR/${INPUT_BASE}_jp.${INPUT_FILE##*.}"
    fi
fi
echo "CacheRoot  = $CACHE_ROOT"
echo "Provider   = $PROVIDER"
echo "Model      = $MODEL"
if [ -n "$GLOSSARY_PATH" ]; then echo "Glossary   = $GLOSSARY_PATH"; fi
if [ -n "$MANUAL" ]; then echo "Manual     = $MANUAL"; fi
echo

echo "Recovery:"
if [ "$NO_RUN" = "0" ]; then
    if [ "$DEV_BUILD" = "1" ]; then cargo run --quiet -- "$@"
    else cargo run --release --quiet -- "$@"
    fi
fi
