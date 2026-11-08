#!/usr/bin/env bash
# Sovereign Codeberg CI Chore-Polish & Release Agent for omni-todos
set -euo pipefail

MODE="${1:---chore-polish}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "==> [omni-todos Codeberg Agent] Mode: $MODE"

run_chore_polish() {
    echo "--> 1. Checking Rust formatting (cargo fmt --all -- --check)..."
    cargo fmt --all -- --check

    echo "--> 2. Running strict Clippy lints (-D warnings)..."
    cargo clippy --all-targets -- -D warnings

    echo "--> 3. Running cargo test..."
    cargo test
    echo "    ✓ omni-todos chore-polish checks passed."
}

run_release() {
    run_chore_polish
    VERSION="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/')"
    TAG="v${VERSION}"
    echo "--> Building release binary for omni-todos ${TAG}..."
    cargo build --release

    mkdir -p dist
    if [ -f target/release/omni-todos ]; then
        cp target/release/omni-todos "dist/omni-todos-${TAG}-$(uname -s)-$(uname -m)"
        shasum -a 256 "dist/omni-todos-${TAG}-$(uname -s)-$(uname -m)" > "dist/omni-todos-${TAG}.sha256"
    fi
    echo "    ✓ Release ${TAG} ready in dist/"
}

case "$MODE" in
    --chore-polish)
        run_chore_polish
        ;;
    --release)
        run_release
        ;;
    *)
        echo "Usage: $0 [--chore-polish | --release]"
        exit 1
        ;;
esac
