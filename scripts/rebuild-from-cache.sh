#!/usr/bin/env sh
# Rebuild an output EPUB from the existing cache without calling the provider API.
#
# Provider/model are auto-detected from <cache>/<input_hash>/translations.jsonl
# (the dominant combination found). Override with --provider / --model.
#
# Usage:
#   chmod +x scripts/rebuild-from-cache.sh
#   scripts/rebuild-from-cache.sh ./book.epub
#   scripts/rebuild-from-cache.sh ./book.epub fixed
#   scripts/rebuild-from-cache.sh ./book.epub reflow
#   scripts/rebuild-from-cache.sh ./book.epub --provider claude
#
# Layout:
#   auto   = use epubicus automatic Kindle fixed-layout detection
#   fixed  = force Kindle fixed-layout metadata
#   reflow = suppress Kindle fixed-layout metadata

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
LAYOUT="auto"
PROVIDER=""
MODEL=""
CACHE_ROOT=""
GLOSSARY=""
DEV_BUILD="0"
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --provider|-p) PROVIDER="$2"; shift 2 ;;
        --model|-m) MODEL="$2"; shift 2 ;;
        --cache-root|-cr) CACHE_ROOT="$2"; shift 2 ;;
        --glossary|-g) GLOSSARY="$2"; shift 2 ;;
        --dev-build) DEV_BUILD="1"; shift ;;
        --no-run) NO_RUN="1"; shift ;;
        --) shift; break ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *)
            if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$1"
            elif [ "$LAYOUT" = "auto" ] && { [ "$1" = "auto" ] || [ "$1" = "fixed" ] || [ "$1" = "reflow" ]; }; then LAYOUT="$1"
            else echo "unexpected argument: $1" >&2; exit 2
            fi
            shift
            ;;
    esac
done

case "$LAYOUT" in
    auto|fixed|reflow) ;;
    *) echo "error: invalid layout: $LAYOUT (auto|fixed|reflow)" >&2; exit 2 ;;
esac

if [ -n "$PROVIDER" ]; then
    case "$PROVIDER" in
        ollama|openai|claude|deepseek) ;;
        *) echo "error: invalid --provider: $PROVIDER" >&2; exit 2 ;;
    esac
fi

if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$PROJECT_ROOT/test/sample.epub"; fi

INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_BASE=${INPUT_FILE%.*}
INPUT_EXT=${INPUT_FILE##*.}
INPUT_EPUB="$INPUT_DIR/$INPUT_FILE"
if [ "$INPUT_BASE" = "$INPUT_FILE" ]; then OUTPUT_EPUB="$INPUT_DIR/${INPUT_FILE}_jp"
else OUTPUT_EPUB="$INPUT_DIR/${INPUT_BASE}_jp.${INPUT_EXT}"
fi

if [ -z "$CACHE_ROOT" ]; then CACHE_ROOT="$PROJECT_ROOT/.cache"; fi

# Compute input_hash (SHA-256 first 16 bytes = 32 hex chars).
sha256_first_16_hex() {
    file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        full=$(sha256sum -- "$file" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        full=$(shasum -a 256 -- "$file" | awk '{print $1}')
    elif command -v openssl >/dev/null 2>&1; then
        full=$(openssl dgst -sha256 -- "$file" | awk '{print $NF}')
    else
        echo "error: need sha256sum / shasum / openssl to compute input_hash" >&2
        exit 2
    fi
    printf '%s' "$full" | tr 'A-Z' 'a-z' | cut -c1-32
}

# Auto-detect provider/model from cache when not given explicitly.
if [ -z "$PROVIDER" ] || [ -z "$MODEL" ]; then
    INPUT_HASH=$(sha256_first_16_hex "$INPUT_EPUB")
    TRANSLATIONS_FILE="$CACHE_ROOT/$INPUT_HASH/translations.jsonl"

    if [ ! -f "$TRANSLATIONS_FILE" ]; then
        echo "error: no cache found at $TRANSLATIONS_FILE" >&2
        echo "  run translate first or specify both --provider and --model" >&2
        exit 2
    fi

    # Find dominant (provider, model) combination using grep + sort + uniq.
    # Each entry is JSON like: {..."provider":"deepseek","model":"deepseek-v4-flash",...}
    pairs=$(grep -oE '"provider":"[^"]+","model":"[^"]+"' "$TRANSLATIONS_FILE" 2>/dev/null || true)
    if [ -z "$pairs" ]; then
        echo "error: cache file has no usable entries: $TRANSLATIONS_FILE" >&2
        exit 2
    fi
    detected_total=$(echo "$pairs" | wc -l | tr -d ' ')
    dominant_line=$(echo "$pairs" | sort | uniq -c | sort -rn | head -1)
    detected_count=$(echo "$dominant_line" | awk '{print $1}')
    dominant_pair=$(echo "$dominant_line" | sed -E 's/^[[:space:]]*[0-9]+[[:space:]]+//')
    detected_provider=$(echo "$dominant_pair" | sed -nE 's/.*"provider":"([^"]+)".*/\1/p')
    detected_model=$(echo "$dominant_pair" | sed -nE 's/.*"model":"([^"]+)".*/\1/p')

    [ -z "$PROVIDER" ] && PROVIDER="$detected_provider"
    [ -z "$MODEL" ] && MODEL="$detected_model"

    echo "Detected from cache: Provider=$PROVIDER, Model=$MODEL ($detected_count/$detected_total entries)"
fi

# Glossary auto-detect.
GLOSSARY_PATH=""
if [ -n "$GLOSSARY" ]; then
    GLOSSARY_PATH="$GLOSSARY"
elif [ -f "$INPUT_DIR/$INPUT_BASE.json" ]; then
    GLOSSARY_PATH="$INPUT_DIR/$INPUT_BASE.json"
fi

set -- translate "$INPUT_EPUB" \
    --provider "$PROVIDER" \
    --model "$MODEL" \
    --cache-root "$CACHE_ROOT" \
    --keep-cache \
    --output "$OUTPUT_EPUB" \
    --partial-from-cache
if [ -n "$GLOSSARY_PATH" ]; then set -- "$@" --glossary "$GLOSSARY_PATH"; fi
case "$LAYOUT" in
    fixed)  set -- "$@" --kindle-fixed-layout ;;
    reflow) set -- "$@" --no-kindle-fixed-layout ;;
esac

echo
echo "InputEpub  = $INPUT_EPUB"
echo "OutputEpub = $OUTPUT_EPUB"
echo "CacheRoot  = $CACHE_ROOT"
echo "Provider   = $PROVIDER"
echo "Model      = $MODEL"
echo "Layout     = $LAYOUT"
if [ -n "$GLOSSARY_PATH" ]; then echo "Glossary   = $GLOSSARY_PATH"; fi
echo

if [ "$NO_RUN" = "0" ]; then
    if [ "$DEV_BUILD" = "1" ]; then cargo run --quiet -- "$@"
    else cargo run --release --quiet -- "$@"
    fi
fi
