#! /bin/bash
set -e

export TERM=xterm-256color

# Statements waiting to be executed
statements=(
    "cargo publish -p $1 --dry-run"
    "cargo publish -p $1"
)

# loop echo and executing statements
for statement in "${statements[@]}"; do
    echo "$(tput setaf 3)$statement$(tput sgr0)"
    eval $statement
    echo
done
