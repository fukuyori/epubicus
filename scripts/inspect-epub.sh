#!/usr/bin/env sh
# Inspect an EPUB and print its table of contents.
#
# Usage:
#   chmod +x scripts/inspect-epub.sh
#   scripts/inspect-epub.sh ./book.epub
#   scripts/inspect-epub.sh ./book.epub --no-run

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

INPUT_PATH=""
NO_RUN="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
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
INPUT_EPUB="$INPUT_DIR/$INPUT_FILE"

invoke_step() {
    name="$1"; shift
    echo
    echo "[$name]"
    if [ "$NO_RUN" = "0" ]; then
        cargo run --release --quiet -- "$@"
    fi
}

echo
echo "InputEpub = $INPUT_EPUB"

invoke_step "inspect" inspect "$INPUT_EPUB"
invoke_step "toc" toc "$INPUT_EPUB"
