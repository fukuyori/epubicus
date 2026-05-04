#!/usr/bin/env sh
# Build the release executable used for direct runs:
#   ./target/release/epubicus

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

cd "$PROJECT_ROOT"
cargo build --release
echo "Built $PROJECT_ROOT/target/release/epubicus"
