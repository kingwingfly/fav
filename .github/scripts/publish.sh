#! /bin/bash
set -e

# Statements waiting to be executed
statements=(
    "cargo login $1"
    "cargo publish -p fav_derive --dry-run"
    "cargo publish -p fav_core --dry-run"
    "cargo publish -p fav_utils --dry-run"
    "cargo publish -p fav_derive"
    "cargo publish -p fav_core"
    "cargo publish -p fav_utils"
)

# loop echo and executing statements
for statement in "${statements[@]}"; do
    echo "$(tput setaf 3)$statement$(tput sgr0)"
    eval $statement
    echo
done
