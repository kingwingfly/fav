#!/usr/bin/env bash
set -euo pipefail

run_step() {
    printf '\033[33m==> %s\033[0m\n' "$*"
    "$@"
    echo
}

run_step cargo fetch --locked
run_step cargo fmt --all -- --check
run_step cargo clippy --workspace --all-targets --all-features -- -D warnings
run_step cargo test --workspace --locked
