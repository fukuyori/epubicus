#!/usr/bin/env sh
# Estimate API usage for a selected content-file range without calling the provider.
# Works for any supported provider (ollama, openai, claude, deepseek).
#
# Usage:
#   chmod +x scripts/usage.sh
#   scripts/usage.sh ./book.epub 9 9 --provider deepseek
#   scripts/usage.sh ./book.epub 9 9 --provider claude
#   scripts/usage.sh ./book.epub 9 9 --provider deepseek --model deepseek-v4-pro
#
# Arguments:
#   1: input EPUB path.
#   2: first 1-based OPF spine number. Required.
#   3: last 1-based OPF spine number. Required.
#   --provider: required. One of ollama / openai / claude / deepseek.
#
# This command does not call the provider API. It only prints estimated request
# counts and input/output tokens for the selected range.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
FROM=""
TO=""
PROVIDER=""
MODEL=""
CACHE_ROOT=""
GLOSSARY=""
DEV_BUILD="0"
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            awk 'NR==1 && /^#!/ {next} /^#/ {sub(/^# ?/,""); print; next} {exit}' "$0"
            exit 0
            ;;
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
            elif [ -z "$FROM" ]; then FROM="$1"
            elif [ -z "$TO" ]; then TO="$1"
            else echo "unexpected argument: $1" >&2; exit 2
            fi
            shift
            ;;
    esac
done

if [ -z "$PROVIDER" ]; then
    echo "error: --provider is required (ollama|openai|claude|deepseek)" >&2
    exit 2
fi

case "$PROVIDER" in
    ollama|openai|claude|deepseek) ;;
    *) echo "error: invalid --provider: $PROVIDER" >&2; exit 2 ;;
esac

if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$PROJECT_ROOT/test/sample.epub"; fi
if [ -z "$FROM" ] || [ -z "$TO" ] || [ "$FROM" -le 0 ] || [ "$TO" -le 0 ] 2>/dev/null; then
    echo "error: start and end spine numbers are required" >&2
    echo "  e.g. scripts/usage.sh ./book.epub 5 5 --provider deepseek" >&2
    exit 2
fi

INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_BASE=${INPUT_FILE%.*}
INPUT_EPUB="$INPUT_DIR/$INPUT_FILE"

# Provider-specific Model defaults (mirrors translate-<provider>.sh).
if [ -z "$MODEL" ]; then
    case "$PROVIDER" in
        ollama)   MODEL="qwen3:14b" ;;
        openai)   MODEL="gpt-5-mini" ;;
        claude)   MODEL="claude-sonnet-4-5" ;;
        deepseek) MODEL="deepseek-v4-flash" ;;
    esac
fi

if [ -z "$CACHE_ROOT" ]; then
    CACHE_ROOT="$PROJECT_ROOT/.cache"
fi

# Auto-detect glossary next to the EPUB.
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
    --from "$FROM" \
    --to "$TO" \
    --usage-only
if [ -n "$GLOSSARY_PATH" ]; then set -- "$@" --glossary "$GLOSSARY_PATH"; fi

echo
echo "InputEpub  = $INPUT_EPUB"
echo "Provider   = $PROVIDER"
echo "Model      = $MODEL"
echo "CacheRoot  = $CACHE_ROOT"
echo "Range      = $FROM..$TO"
if [ -n "$GLOSSARY_PATH" ]; then echo "Glossary   = $GLOSSARY_PATH"; fi
echo

if [ "$NO_RUN" = "0" ]; then
    if [ "$DEV_BUILD" = "1" ]; then
        cargo run -- "$@"
    else
        cargo run --release -- "$@"
    fi
fi
