#!/usr/bin/env sh
# Recover unfinished OpenAI Batch work locally, then rebuild the EPUB.
#
# The script uses the shared .cache by default and does not stop running
# epubicus processes. Use --no-run first when checking a command sequence.
#
# Usage:
#   chmod +x scripts/batch-recover-local.sh
#   scripts/batch-recover-local.sh ./book.epub --no-run
#   scripts/batch-recover-local.sh ./book.epub --local-model qwen3:14b --limit 100
#   scripts/batch-recover-local.sh ./book.epub --strict-verify

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
CACHE_ROOT=""
OUTPUT=""
GLOSSARY=""
BATCH_MODEL="gpt-5-mini"
LOCAL_PROVIDER="ollama"
LOCAL_MODEL="qwen3:14b"
LIMIT="100"
PRIORITY="short-first"
SKIP_FETCH_IMPORT="0"
SKIP_LOCAL="0"
SKIP_REBUILD="0"
STRICT_VERIFY="0"
EPUBICUS_EXE=""
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --cache-root) CACHE_ROOT="$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --glossary) GLOSSARY="$2"; shift 2 ;;
        --batch-model) BATCH_MODEL="$2"; shift 2 ;;
        --local-provider) LOCAL_PROVIDER="$2"; shift 2 ;;
        --local-model) LOCAL_MODEL="$2"; shift 2 ;;
        --limit) LIMIT="$2"; shift 2 ;;
        --priority) PRIORITY="$2"; shift 2 ;;
        --skip-fetch-import) SKIP_FETCH_IMPORT="1"; shift ;;
        --skip-local) SKIP_LOCAL="1"; shift ;;
        --skip-rebuild) SKIP_REBUILD="1"; shift ;;
        --strict-verify) STRICT_VERIFY="1"; shift ;;
        --epubicus-exe) EPUBICUS_EXE="$2"; shift 2 ;;
        --no-run) NO_RUN="1"; shift ;;
        --) shift; break ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *)
            if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$1"; shift
            else echo "unexpected argument: $1" >&2; exit 2; fi
            ;;
    esac
done

case "$LOCAL_PROVIDER" in
    ollama|openai|claude) ;;
    *) echo "error: invalid --local-provider: $LOCAL_PROVIDER" >&2; exit 2 ;;
esac

case "$PRIORITY" in
    page-order|failed-first|hard-first|short-first|oldest-first) ;;
    *) echo "error: invalid --priority: $PRIORITY" >&2; exit 2 ;;
esac

if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$PROJECT_ROOT/test/sample.epub"; fi
INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_BASE=${INPUT_FILE%.*}
INPUT_EXT=${INPUT_FILE##*.}
INPUT_EPUB="$INPUT_DIR/$INPUT_FILE"

if [ -z "$CACHE_ROOT" ]; then CACHE_ROOT="$PROJECT_ROOT/.cache"; fi

if [ -z "$OUTPUT" ]; then
    if [ "$INPUT_BASE" = "$INPUT_FILE" ]; then OUTPUT="$INPUT_DIR/${INPUT_FILE}_jp"
    else OUTPUT="$INPUT_DIR/${INPUT_BASE}_jp.${INPUT_EXT}"
    fi
fi

GLOSSARY_PATH=""
if [ -n "$GLOSSARY" ]; then
    GLOSSARY_PATH="$GLOSSARY"
elif [ -f "$INPUT_DIR/$INPUT_BASE.json" ]; then
    GLOSSARY_PATH="$INPUT_DIR/$INPUT_BASE.json"
fi

# Pick epubicus binary.
EPUBICUS=""
if [ -n "$EPUBICUS_EXE" ]; then
    EPUBICUS="$EPUBICUS_EXE"
elif [ -f "$PROJECT_ROOT/target/debug/epubicus" ]; then
    EPUBICUS="$PROJECT_ROOT/target/debug/epubicus"
elif [ -f "$PROJECT_ROOT/target/release/epubicus" ]; then
    EPUBICUS="$PROJECT_ROOT/target/release/epubicus"
fi

run_epubicus() {
    if [ -n "$EPUBICUS" ]; then
        "$EPUBICUS" "$@"
    else
        cargo run --quiet -- "$@"
    fi
}

invoke_step() {
    name="$1"; shift
    continue_on_error="$1"; shift
    echo
    echo "[$name]"
    if [ "$NO_RUN" = "1" ]; then return 0; fi
    if run_epubicus "$@"; then
        return 0
    fi
    rc=$?
    if [ "$continue_on_error" = "1" ]; then
        echo "warning: $name failed with exit code $rc; continuing." >&2
        return 0
    fi
    echo "error: $name failed with exit code $rc" >&2
    exit "$rc"
}

# Common args used by all batch subcommands.
common_args() {
    set -- "$INPUT_EPUB" --cache-root "$CACHE_ROOT"
    if [ -n "$GLOSSARY_PATH" ]; then set -- "$@" --glossary "$GLOSSARY_PATH"; fi
    printf '%s\n' "$@"
}

echo
echo "InputEpub     = $INPUT_EPUB"
echo "OutputEpub    = $OUTPUT"
echo "CacheRoot     = $CACHE_ROOT"
echo "BatchModel    = $BATCH_MODEL"
echo "LocalProvider = $LOCAL_PROVIDER"
echo "LocalModel    = $LOCAL_MODEL"
if [ -n "$GLOSSARY_PATH" ]; then echo "Glossary      = $GLOSSARY_PATH"; fi

# Read common args into shell positional via newline trick.
COMMON_TMP=$(common_args)
# Split COMMON_TMP back into args using newline as IFS for the duration of each call.

# Step 1: health before
{
    OLD_IFS=$IFS; IFS='
'
    set -- batch health $COMMON_TMP
    IFS=$OLD_IFS
    invoke_step "health before" "0" "$@"
}

if [ "$SKIP_FETCH_IMPORT" = "0" ]; then
    OLD_IFS=$IFS; IFS='
'
    set -- batch fetch $COMMON_TMP
    IFS=$OLD_IFS
    invoke_step "fetch" "0" "$@"

    OLD_IFS=$IFS; IFS='
'
    set -- batch import $COMMON_TMP
    IFS=$OLD_IFS
    invoke_step "import" "0" "$@"
fi

if [ "$SKIP_LOCAL" = "0" ]; then
    OLD_IFS=$IFS; IFS='
'
    set -- batch reroute-local $COMMON_TMP
    IFS=$OLD_IFS
    invoke_step "reroute local" "0" "$@" --provider openai --model "$BATCH_MODEL" --remaining --priority "$PRIORITY"

    OLD_IFS=$IFS; IFS='
'
    set -- batch translate-local $COMMON_TMP
    IFS=$OLD_IFS
    if [ "$LIMIT" -gt 0 ] 2>/dev/null; then
        invoke_step "translate local" "0" "$@" --provider "$LOCAL_PROVIDER" --model "$LOCAL_MODEL" --priority "$PRIORITY" --limit "$LIMIT"
    else
        invoke_step "translate local" "0" "$@" --provider "$LOCAL_PROVIDER" --model "$LOCAL_MODEL" --priority "$PRIORITY"
    fi
fi

OLD_IFS=$IFS; IFS='
'
set -- batch verify $COMMON_TMP
IFS=$OLD_IFS
if [ "$STRICT_VERIFY" = "1" ]; then
    invoke_step "verify" "0" "$@"
else
    invoke_step "verify" "1" "$@"
fi

OLD_IFS=$IFS; IFS='
'
set -- batch health $COMMON_TMP
IFS=$OLD_IFS
invoke_step "health after" "0" "$@"

if [ "$SKIP_REBUILD" = "0" ]; then
    set -- translate "$INPUT_EPUB" \
        --cache-root "$CACHE_ROOT" \
        --provider openai \
        --model "$BATCH_MODEL" \
        --partial-from-cache \
        --keep-cache \
        --output "$OUTPUT"
    if [ -n "$GLOSSARY_PATH" ]; then set -- "$@" --glossary "$GLOSSARY_PATH"; fi
    invoke_step "rebuild epub" "0" "$@"
fi
