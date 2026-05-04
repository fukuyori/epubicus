#!/usr/bin/env sh
# epubicus OpenAI Batch API environment template.
#
# Usage:
#   cp scripts/openai-batch-env.template.sh scripts/openai-batch-env.sh
#   chmod +x scripts/openai-batch-env.sh
#   export OPENAI_API_KEY="..."
#   scripts/openai-batch-env.sh ./book.epub
#   scripts/openai-batch-env.sh ./book.epub -- --glossary ./glossary.json

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
FROM="0"
TO="0"
MODEL="gpt-5-mini"
POLL_SECS="180"
NO_WAIT="0"
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --from) FROM="$2"; shift 2 ;;
        --to) TO="$2"; shift 2 ;;
        --model) MODEL="$2"; shift 2 ;;
        --poll-secs) POLL_SECS="$2"; shift 2 ;;
        --no-wait) NO_WAIT="1"; shift ;;
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
export CacheRoot="$PROJECT_ROOT/.batch-openai-cache"
AutoGlossary=""
case " $* " in
    *" --glossary "*|*" --glossary="*|*" -g "*) ;;
    *)
        if [ -f "$INPUT_DIR/$INPUT_BASE.json" ]; then
            AutoGlossary="$INPUT_DIR/$INPUT_BASE.json"
        fi
        ;;
esac
export EPUBICUS_PROVIDER="openai"
export EPUBICUS_MODEL="$MODEL"
export EPUBICUS_OPENAI_BASE_URL="https://api.openai.com/v1"
export EPUBICUS_STYLE="essay"
export EPUBICUS_TEMPERATURE="0.3"
export EPUBICUS_TIMEOUT_SECS="900"
export EPUBICUS_RETRIES="3"
export EPUBICUS_MAX_CHARS_PER_REQUEST="3500"
export EPUBICUS_CONCURRENCY="1"
export EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE="true"

if [ -z "${OPENAI_API_KEY:-}" ]; then
    echo "warning: OPENAI_API_KEY is not set" >&2
fi

invoke_epubicus_openai_batch() {
    set -- run "$InputEpub" --provider openai --model "$EPUBICUS_MODEL" --cache-root "$CacheRoot" --force-prepare --poll-secs "$POLL_SECS" --output "$OutputEpub" "$@"
    if [ -n "$AutoGlossary" ]; then set -- "$@" --glossary "$AutoGlossary"; fi
    if [ "$NO_WAIT" = "0" ]; then set -- "$@" --wait; fi
    if [ "$FROM" -gt 0 ]; then set -- "$@" --from "$FROM"; fi
    if [ "$TO" -gt 0 ]; then set -- "$@" --to "$TO"; fi
    cargo run --release -- batch "$@"
}

invoke_epubicus_openai_batch_status() {
    cargo run --release -- batch status "$InputEpub" --cache-root "$CacheRoot"
}

invoke_epubicus_openai_batch_verify() {
    cargo run --release -- batch verify "$InputEpub" --cache-root "$CacheRoot"
}

echo
echo "InputEpub  = $InputEpub"
echo "OutputEpub = $OutputEpub"
echo "CacheRoot  = $CacheRoot"
echo "Model      = $EPUBICUS_MODEL"
if [ -n "$AutoGlossary" ]; then
    echo "Glossary   = $AutoGlossary"
fi
if [ "$#" -gt 0 ]; then
    echo "ExtraArgs  = $*"
fi
echo
echo "Batch conversion:"
echo "invoke_epubicus_openai_batch"
echo

if [ "$NO_RUN" = "0" ]; then
    invoke_epubicus_openai_batch "$@"
fi

