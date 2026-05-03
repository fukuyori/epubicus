#!/usr/bin/env sh
# epubicus local Ollama environment template.
#
# Usage:
#   cp scripts/local-ollama-env.template.sh scripts/local-ollama-env.sh
#   chmod +x scripts/local-ollama-env.sh
#   scripts/local-ollama-env.sh ./book.epub
#
# Modes:
#   scripts/local-ollama-env.sh ./book.epub --mode page --from 3 --to 3
#   scripts/local-ollama-env.sh ./book.epub --mode cache
#   . scripts/local-ollama-env.sh ./book.epub --no-run
#   scripts/local-ollama-env.sh ./book.epub -- --glossary ./glossary.json

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
MODE="full"
FROM="3"
TO="3"
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --mode)
            MODE="$2"
            shift 2
            ;;
        --from)
            FROM="$2"
            shift 2
            ;;
        --to)
            TO="$2"
            shift 2
            ;;
        --no-run)
            NO_RUN="1"
            shift
            ;;
        --)
            shift
            break
            ;;
        -*)
            echo "unknown option: $1" >&2
            return 2 2>/dev/null || exit 2
            ;;
        *)
            if [ -z "$INPUT_PATH" ]; then
                INPUT_PATH="$1"
                shift
            else
                echo "unexpected argument: $1" >&2
                return 2 2>/dev/null || exit 2
            fi
            ;;
    esac
done

if [ -z "$INPUT_PATH" ]; then
    INPUT_PATH="$PROJECT_ROOT/test/sample.epub"
fi

INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_BASE=${INPUT_FILE%.*}
INPUT_EXT=${INPUT_FILE##*.}
if [ "$INPUT_BASE" = "$INPUT_FILE" ]; then
    OUTPUT_FILE="${INPUT_FILE}_jp"
else
    OUTPUT_FILE="${INPUT_BASE}_jp.${INPUT_EXT}"
fi

export InputEpub="$INPUT_DIR/$INPUT_FILE"
export OutputEpub="$INPUT_DIR/$OUTPUT_FILE"
export CacheRoot="$PROJECT_ROOT/.local-ollama-cache"

export EPUBICUS_PROVIDER="ollama"
export EPUBICUS_MODEL="qwen3:14b"
export EPUBICUS_OLLAMA_HOST="http://localhost:11434"
export EPUBICUS_STYLE="essay"
export EPUBICUS_TEMPERATURE="0.3"
export EPUBICUS_NUM_CTX="8192"
export EPUBICUS_TIMEOUT_SECS="900"
export EPUBICUS_RETRIES="3"
export EPUBICUS_MAX_CHARS_PER_REQUEST="3500"
export EPUBICUS_CONCURRENCY="2"
export EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE="true"

show_epubicus_local_commands() {
    echo
    echo "InputEpub  = $InputEpub"
    echo "OutputEpub = $OutputEpub"
    echo "CacheRoot  = $CacheRoot"
    if [ "$#" -gt 0 ]; then
        echo "ExtraArgs  = $*"
    fi
    echo
    echo "Local page-range check:"
    echo "invoke_epubicus_local_page_check"
    echo "cargo run --release -- translate \"\$InputEpub\" --cache-root \"\$CacheRoot\" --from $FROM --to $TO --keep-cache --output \"\$OutputEpub\" --passthrough-on-validation-failure"
    echo
    echo "Local full conversion:"
    echo "invoke_epubicus_local_full"
    echo "cargo run --release -- translate \"\$InputEpub\" --cache-root \"\$CacheRoot\" --keep-cache --output \"\$OutputEpub\" --passthrough-on-validation-failure"
    echo
    echo "Assemble from cache only:"
    echo "invoke_epubicus_assemble_from_cache"
    echo "cargo run --release -- translate \"\$InputEpub\" --cache-root \"\$CacheRoot\" --partial-from-cache --keep-cache --output \"\$OutputEpub\""
    echo
}

invoke_epubicus_local_page_check() {
    cargo run --release -- translate "$InputEpub" \
        --cache-root "$CacheRoot" \
        --from "$FROM" \
        --to "$TO" \
        --keep-cache \
        --output "$OutputEpub" \
        --passthrough-on-validation-failure \
        "$@"
}

invoke_epubicus_local_full() {
    cargo run --release -- translate "$InputEpub" \
        --cache-root "$CacheRoot" \
        --keep-cache \
        --output "$OutputEpub" \
        --passthrough-on-validation-failure \
        "$@"
}

invoke_epubicus_assemble_from_cache() {
    cargo run --release -- translate "$InputEpub" \
        --cache-root "$CacheRoot" \
        --partial-from-cache \
        --keep-cache \
        --output "$OutputEpub" \
        "$@"
}

show_epubicus_local_commands "$@"

if [ "$NO_RUN" = "0" ]; then
    case "$MODE" in
        page) invoke_epubicus_local_page_check "$@" ;;
        cache) invoke_epubicus_assemble_from_cache "$@" ;;
        full) invoke_epubicus_local_full "$@" ;;
        *)
            echo "unknown mode: $MODE" >&2
            return 2 2>/dev/null || exit 2
            ;;
    esac
fi

