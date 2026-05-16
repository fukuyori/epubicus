#!/usr/bin/env sh
# Scan a translated EPUB for untranslated-looking blocks and optionally recover.
#
# Usage (--provider is required):
#   chmod +x scripts/scan-and-recover.sh
#   scripts/scan-and-recover.sh ./book.epub --provider deepseek            # scan + recover
#   scripts/scan-and-recover.sh ./book.epub --provider deepseek --scan-only  # scan only (no API call)
#   scripts/scan-and-recover.sh ./book.epub --provider claude
#   scripts/scan-and-recover.sh ./book.epub ./book_jp.epub --provider deepseek --no-run

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
OUTPUT_PATH=""
PROVIDER=""
MODEL=""
CACHE_ROOT=""
GLOSSARY=""
LIMIT="0"
SCAN_ONLY="0"
NO_REBUILD="0"
KINDLE_FIXED="0"
NO_KINDLE_FIXED="0"
EPUBICUS_EXE=""
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            while IFS= read -r ln; do
                case "$ln" in
                    '#!'*) ;;
                    '#'*) r=${ln#\#}; printf '%s\n' "${r# }" ;;
                    *) break ;;
                esac
            done < "$0"
            exit 0
            ;;
        --provider|-p) PROVIDER="$2"; shift 2 ;;
        --model|-m) MODEL="$2"; shift 2 ;;
        --cache-root|-cr) CACHE_ROOT="$2"; shift 2 ;;
        --glossary|-g) GLOSSARY="$2"; shift 2 ;;
        --limit|-l) LIMIT="$2"; shift 2 ;;
        --scan-only) SCAN_ONLY="1"; shift ;;
        --no-rebuild) NO_REBUILD="1"; shift ;;
        --kindle-fixed-layout) KINDLE_FIXED="1"; shift ;;
        --no-kindle-fixed-layout) NO_KINDLE_FIXED="1"; shift ;;
        --epubicus-exe) EPUBICUS_EXE="$2"; shift 2 ;;
        --no-run) NO_RUN="1"; shift ;;
        --) shift; break ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *)
            if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$1"
            elif [ -z "$OUTPUT_PATH" ]; then OUTPUT_PATH="$1"
            else echo "unexpected argument: $1" >&2; exit 2
            fi
            shift
            ;;
    esac
done

if [ "$KINDLE_FIXED" = "1" ] && [ "$NO_KINDLE_FIXED" = "1" ]; then
    echo "error: --kindle-fixed-layout and --no-kindle-fixed-layout cannot be used together" >&2
    exit 2
fi

if [ -z "$PROVIDER" ]; then
    echo "error: --provider is required (ollama|openai|claude|deepseek)" >&2
    exit 2
fi
case "$PROVIDER" in
    ollama|openai|claude|deepseek) ;;
    *) echo "error: invalid --provider: $PROVIDER" >&2; exit 2 ;;
esac

if [ -z "$INPUT_PATH" ]; then INPUT_PATH="$PROJECT_ROOT/test/sample.epub"; fi
INPUT_DIR=$(CDPATH= cd -- "$(dirname -- "$INPUT_PATH")" && pwd)
INPUT_FILE=$(basename -- "$INPUT_PATH")
INPUT_BASE=${INPUT_FILE%.*}
INPUT_EXT=${INPUT_FILE##*.}
INPUT_EPUB="$INPUT_DIR/$INPUT_FILE"

if [ -z "$OUTPUT_PATH" ]; then
    if [ "$INPUT_BASE" = "$INPUT_FILE" ]; then OUTPUT_PATH="$INPUT_DIR/${INPUT_FILE}_jp"
    else OUTPUT_PATH="$INPUT_DIR/${INPUT_BASE}_jp.${INPUT_EXT}"
    fi
fi

if [ -z "$CACHE_ROOT" ]; then CACHE_ROOT="$PROJECT_ROOT/.cache"; fi

if [ -z "$MODEL" ]; then
    case "$PROVIDER" in
        ollama)   MODEL="qwen3:14b" ;;
        openai)   MODEL="gpt-5-mini" ;;
        claude)   MODEL="claude-sonnet-4-5" ;;
        deepseek) MODEL="deepseek-v4-flash" ;;
    esac
fi

# Glossary auto-detect.
GLOSSARY_PATH=""
if [ -n "$GLOSSARY" ]; then
    GLOSSARY_PATH="$GLOSSARY"
elif [ -f "$INPUT_DIR/$INPUT_BASE.json" ]; then
    GLOSSARY_PATH="$INPUT_DIR/$INPUT_BASE.json"
fi

set -- scan-recovery "$INPUT_EPUB" "$OUTPUT_PATH" \
    --provider "$PROVIDER" \
    --model "$MODEL" \
    --cache-root "$CACHE_ROOT"

if [ "$LIMIT" -gt 0 ] 2>/dev/null; then set -- "$@" --limit "$LIMIT"; fi

if [ "$SCAN_ONLY" = "0" ]; then
    set -- "$@" --recover
    if [ "$NO_REBUILD" = "0" ]; then set -- "$@" --rebuild; fi
fi

if [ -n "$GLOSSARY_PATH" ]; then set -- "$@" --glossary "$GLOSSARY_PATH"; fi
if [ "$KINDLE_FIXED" = "1" ]; then set -- "$@" --kindle-fixed-layout; fi
if [ "$NO_KINDLE_FIXED" = "1" ]; then set -- "$@" --no-kindle-fixed-layout; fi

echo
echo "InputEpub  = $INPUT_EPUB"
echo "OutputEpub = $OUTPUT_PATH"
echo "CacheRoot  = $CACHE_ROOT"
echo "Provider   = $PROVIDER"
echo "Model      = $MODEL"
if [ -n "$GLOSSARY_PATH" ]; then echo "Glossary   = $GLOSSARY_PATH"; fi
echo

if [ "$NO_RUN" = "0" ]; then
    if [ -n "$EPUBICUS_EXE" ]; then "$EPUBICUS_EXE" "$@"
    else cargo run --release --quiet -- "$@"
    fi
fi
