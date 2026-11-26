#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if ! command -v sea-orm-cli >/dev/null 2>&1; then
    echo "error: sea-orm-cli not found on PATH (install with: cargo binstall sea-orm-cli)" >&2
    exit 1
fi

target_dir="${SCRIPT_DIR}/fav_bili/src/entity/entity_inner"
target_parent="$(dirname "$target_dir")"

tmp_dir="$(mktemp -d)"
stage_dir="$(mktemp -d "${target_parent}/.entity_inner.tmp.XXXXXX")"
backup_dir="${target_parent}/.entity_inner.bak.$$"

cleanup() {
    if [[ ! -d "$target_dir" && -d "$backup_dir" ]]; then
        mv "$backup_dir" "$target_dir"
    fi
    rm -rf "$tmp_dir" "$stage_dir" "$backup_dir"
}
trap cleanup EXIT

tmp_db="${tmp_dir}/schema.db"
export DATABASE_URL="sqlite://${tmp_db}?mode=rwc"

sea-orm-cli migrate refresh -v
sea-orm-cli generate entity --expanded-format -o "$stage_dir"

if [[ ! -f "${stage_dir}/mod.rs" ]]; then
    echo "error: entity generation did not produce mod.rs" >&2
    exit 1
fi

if [[ -d "$target_dir" ]]; then
    mv "$target_dir" "$backup_dir"
fi
mv "$stage_dir" "$target_dir"
rm -rf "$backup_dir"

echo "Regenerated ${target_dir} successfully."
