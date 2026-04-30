#!/usr/bin/env sh
# epubicus Claude normal API environment template.
#
# Usage:
#   cp scripts/claude-env.template.sh scripts/claude-env.sh
#   chmod +x scripts/claude-env.sh
#   export ANTHROPIC_API_KEY="..."
#   scripts/claude-env.sh ./book.epub
#   scripts/claude-env.sh ./book.epub -- --glossary ./glossary.json

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
FROM="0"
TO="0"
MODEL="claude-sonnet-4-5"
CONCURRENCY="1"
USAGE_ONLY="0"
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --from) FROM="$2"; shift 2 ;;
        --to) TO="$2"; shift 2 ;;
        --model) MODEL="$2"; shift 2 ;;
        --concurrency) CONCURRENCY="$2"; shift 2 ;;
        --usage-only) USAGE_ONLY="1"; shift ;;
        --no-run) NO_RUN="1"; shift ;;
        --) shift; break ;;
        -*) echo "unknown option: $1" >&2; return 2 2>/dev/null || exit 2 ;;
        *)
            if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$1"; shift; else echo "unexpected argument: $1" >&2; return 2 2>/dev/null || exit 2; fi
            ;;
    esac
done

if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$PROJECT_ROOT/test/sample.epub"; fi
INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_BASE=${INPUT_FILE%.*}
INPUT_EXT=${INPUT_FILE##*.}
if [ "$INPUT_BASE" = "$INPUT_FILE" ]; then OUTPUT_FILE="${INPUT_FILE}_jp"; else OUTPUT_FILE="${INPUT_BASE}_jp.${INPUT_EXT}"; fi

export InputEpub="$INPUT_DIR/$INPUT_FILE"
export OutputEpub="$INPUT_DIR/$OUTPUT_FILE"
export CacheRoot="$PROJECT_ROOT/.claude-cache"
export EPUBICUS_PROVIDER="claude"
export EPUBICUS_MODEL="$MODEL"
export EPUBICUS_CLAUDE_BASE_URL="https://api.anthropic.com/v1"
export EPUBICUS_STYLE="essay"
export EPUBICUS_TEMPERATURE="0.3"
export EPUBICUS_TIMEOUT_SECS="900"
export EPUBICUS_RETRIES="3"
export EPUBICUS_MAX_CHARS_PER_REQUEST="3500"
export EPUBICUS_CONCURRENCY="$CONCURRENCY"
export EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE="true"

if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
    echo "warning: ANTHROPIC_API_KEY is not set" >&2
fi

invoke_epubicus_claude() {
    set -- translate "$InputEpub" --cache-root "$CacheRoot" --keep-cache --output "$OutputEpub" "$@"
    if [ "$FROM" -gt 0 ]; then set -- "$@" --from "$FROM"; fi
    if [ "$TO" -gt 0 ]; then set -- "$@" --to "$TO"; fi
    if [ "$USAGE_ONLY" = "1" ]; then set -- "$@" --usage-only; fi
    cargo run -- "$@"
}

echo
echo "InputEpub  = $InputEpub"
echo "OutputEpub = $OutputEpub"
echo "CacheRoot  = $CacheRoot"
echo "Model      = $EPUBICUS_MODEL"
if [ "$#" -gt 0 ]; then
    echo "ExtraArgs  = $*"
fi
echo
echo "Normal Claude conversion:"
echo "invoke_epubicus_claude"
echo

if [ "$NO_RUN" = "0" ]; then
    invoke_epubicus_claude "$@"
fi
