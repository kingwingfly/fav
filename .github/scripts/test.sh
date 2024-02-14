#! /bin/bash
set -e

export TERM=xterm-256color

# Statements waiting to be executed
statements=(
    "cargo fetch --locked"
    "cargo clippy --all-features --all-targets -- -D warnings"
    # fav_core
    "cargo test -p fav_core"
    "cargo doc --no-deps -p fav_core"
    "cargo test -p fav_derive"
    # fav_utils_ng
    "cargo test -p fav_utils_ng"
    "cargo doc --no-deps -p fav_utils_ng"
)

# loop echo and executing statements
for statement in "${statements[@]}"; do
    echo "$(tput setaf 3)$statement$(tput sgr0)"
    eval $statement
    echo
done
