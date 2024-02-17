#! /bin/bash
set -e

export TERM=xterm-256color

# Statements waiting to be executed
statements=(
    "cargo clippy --all-features --all-targets -p $1 -- -D warnings"
    "cargo test -p $1"
    "cargo doc --no-deps -p $1"
    "cargo publish -p $1 --dry-run"
)

# loop echo and executing statements
for statement in "${statements[@]}"; do
    echo "$(tput setaf 3)$statement$(tput sgr0)"
    eval $statement
    echo
done
