#!/usr/bin/env sh
# Create glossary candidate files next to an EPUB.
#
# Given:
#   /path/to/sample.epub
#
# This writes:
#   /path/to/sample.json
#   /path/to/sample.md
#
# Usage:
#   chmod +x scripts/create-glossary.sh
#   scripts/create-glossary.sh ./test/sample.epub
#   scripts/create-glossary.sh ./test/sample.epub --force
#   scripts/create-glossary.sh ./test/sample.epub --no-run
#   scripts/create-glossary.sh ./test/sample.epub --dev-build

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
MIN_OCCURRENCES="3"
MAX_ENTRIES="200"
EPUBICUS_EXE=""
FORCE="0"
DEV_BUILD="0"
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --min-occurrences) MIN_OCCURRENCES="$2"; shift 2 ;;
        --max-entries) MAX_ENTRIES="$2"; shift 2 ;;
        --epubicus-exe) EPUBICUS_EXE="$2"; shift 2 ;;
        --force) FORCE="1"; shift ;;
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

if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$PROJECT_ROOT/test/sample.epub"; fi
INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_BASE=${INPUT_FILE%.*}
INPUT_EPUB="$INPUT_DIR/$INPUT_FILE"

OUTPUT_JSON="$INPUT_DIR/$INPUT_BASE.json"
OUTPUT_MD="$INPUT_DIR/$INPUT_BASE.md"

if [ "$FORCE" = "0" ]; then
    existing=""
    if [ -f "$OUTPUT_JSON" ]; then existing="$existing $OUTPUT_JSON"; fi
    if [ -f "$OUTPUT_MD" ]; then existing="$existing $OUTPUT_MD"; fi
    if [ -n "$existing" ]; then
        echo "warning: output file already exists; leaving it unchanged. Use --force to overwrite:$existing" >&2
        exit 0
    fi
fi

echo
echo "InputEpub = $INPUT_EPUB"
echo "JSON      = $OUTPUT_JSON"
echo "Markdown  = $OUTPUT_MD"
echo

set -- glossary "$INPUT_EPUB" \
    --output "$OUTPUT_JSON" \
    --review-prompt "$OUTPUT_MD" \
    --min-occurrences "$MIN_OCCURRENCES" \
    --max-entries "$MAX_ENTRIES"

if [ "$NO_RUN" = "1" ]; then exit 0; fi

if [ -n "$EPUBICUS_EXE" ]; then
    "$EPUBICUS_EXE" "$@"
elif [ "$DEV_BUILD" = "1" ]; then
    cargo run --quiet -- "$@"
else
    cargo run --release --quiet -- "$@"
fi
