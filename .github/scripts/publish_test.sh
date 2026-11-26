#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <package>" >&2
    exit 2
fi

pkg="$1"

run_step() {
    printf '\033[33m==> %s\033[0m\n' "$*"
    "$@"
    echo
}

run_step cargo fmt --all -- --check
run_step cargo clippy --all-features --all-targets -p "$pkg" -- -D warnings
run_step cargo test --locked -p "$pkg"
run_step cargo doc --no-deps --locked -p "$pkg"
run_step cargo publish --locked -p "$pkg" --dry-run
