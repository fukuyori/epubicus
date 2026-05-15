#!/usr/bin/env sh
# Clear all known epubicus cache roots.
#
# Removes both the OS-default cache and the project-local .cache (plus legacy
# provider-specific caches if they still exist). Does not stop running epubicus
# processes; locked files are reported and left for a later cleanup.
#
# Usage:
#   chmod +x scripts/clear-all-caches.sh
#   scripts/clear-all-caches.sh --dry-run
#   scripts/clear-all-caches.sh --yes
#   scripts/clear-all-caches.sh --include ./some-other-cache --yes

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

DRY_RUN="0"
YES="0"
INCLUDE_LIST=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN="1"; shift ;;
        --yes) YES="1"; shift ;;
        --include) INCLUDE_LIST="$INCLUDE_LIST
$2"; shift 2 ;;
        --) shift; break ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *) echo "unexpected argument: $1" >&2; exit 2 ;;
    esac
done

default_cache_root() {
    case "$(uname -s 2>/dev/null)" in
        Darwin)
            if [ -n "${HOME:-}" ]; then echo "$HOME/Library/Caches/epubicus"; fi
            ;;
        *)
            if [ -n "${XDG_CACHE_HOME:-}" ]; then
                echo "$XDG_CACHE_HOME/epubicus"
            elif [ -n "${HOME:-}" ]; then
                echo "$HOME/.cache/epubicus"
            fi
            ;;
    esac
}

CANDIDATES="
$(default_cache_root)
$PROJECT_ROOT/.cache
$PROJECT_ROOT/.epubicus-cache
$PROJECT_ROOT/.openai-cache
$PROJECT_ROOT/.batch-openai-cache
$PROJECT_ROOT/.local-ollama-cache
$PROJECT_ROOT/.claude-cache
$PROJECT_ROOT/.deepseek-cache
$INCLUDE_LIST
"

# Build de-duplicated, existing-only roots list.
ROOTS=""
OLD_IFS=$IFS; IFS='
'
for path in $CANDIDATES; do
    [ -z "$path" ] && continue
    [ ! -d "$path" ] && continue
    resolved=$(CDPATH= cd -- "$path" 2>/dev/null && pwd) || continue
    case "$ROOTS" in
        *"$resolved"*) ;;
        *) ROOTS="$ROOTS
$resolved" ;;
    esac
done
IFS=$OLD_IFS

if [ -z "$ROOTS" ]; then
    echo "No epubicus cache roots found."
    exit 0
fi

echo "epubicus cache roots:"
OLD_IFS=$IFS; IFS='
'
for root in $ROOTS; do
    [ -z "$root" ] && continue
    if command -v du >/dev/null 2>&1; then
        size=$(du -sh "$root" 2>/dev/null | awk '{print $1}')
    else
        size="?"
    fi
    echo "  $root  ($size)"
done
IFS=$OLD_IFS

if [ "$DRY_RUN" = "1" ]; then
    echo "Dry run only. Nothing was deleted."
    exit 0
fi

if [ "$YES" = "0" ]; then
    printf "Delete all cache roots listed above? Type yes: " >&2
    IFS= read -r answer
    if [ "$answer" != "yes" ]; then
        echo "Cancelled."
        exit 1
    fi
fi

deleted=0
failed=0
OLD_IFS=$IFS; IFS='
'
for root in $ROOTS; do
    [ -z "$root" ] && continue
    if rm -rf -- "$root" 2>/dev/null; then
        echo "Deleted: $root"
        deleted=$((deleted + 1))
    else
        echo "warning: failed to delete: $root" >&2
        failed=$((failed + 1))
    fi
done
IFS=$OLD_IFS

echo "Done. Deleted $deleted cache root(s); failed $failed."
if [ "$failed" -gt 0 ]; then exit 2; fi
