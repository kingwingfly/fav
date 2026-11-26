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

run_step cargo publish --locked -p "$pkg"
