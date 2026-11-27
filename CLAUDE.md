# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`fav` is a CLI that backs up Bilibili favorites and followed UPs: it authenticates as one or more
accounts, fetches metadata for followed UPs / favorite sets / medias into a local SQLite database,
and downloads and muxes videos. Workspace: `fav_bili` (library + `fav` binary) + `migration` (a thin
`sea-orm-cli` shim).

## Build, lint, test, run

There is **no build-time FFmpeg / `libav*` / `pkgconf` dependency**. `avmux` has been removed;
media muxing runs the `ffmpeg` CLI subprocess at runtime via `FfmpegMuxer`. Runtime tests that
exercise real `ffmpeg` subprocess cleanup expect `ffmpeg` on `PATH`.

```sh
cargo build
cargo run --bin fav -- ls set
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
```

CI (`.github/workflows/test.yaml`, invoking `.github/scripts/test.sh`) runs on `push` and
`pull_request` for `main` and `dev`, installs runtime `ffmpeg`, caches with `Swatinem/rust-cache@v2`,
and gates on `cargo fetch --locked`, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`, and
`cargo test --workspace --locked`.

`[profile.release]` intentionally omits `panic = "abort"` so RAII `Drop` guards cleanly kill child
`ffmpeg` processes, remove staging `.part` files, and release `.fav/fav.lock` on unwind.

If `build.rs` warns `repository path ... is not owned by current user`, `vergen` cannot read git and
`VERGEN_GIT_DESCRIBE` goes unset, so `--version` reports `crates.io`.

## Architecture

`main.rs` → `lib.rs` (`AppContext`, `command.rs`) → `action/*` → (`api.rs` + `payload/*` + `response/*`
for HTTP, `downloader.rs` + `muxer.rs` for streaming download and `ffmpeg` muxing, `reconcile.rs` for
4-layer rename-safe disk reconciliation, `db/*` for storage) → `entity/entity_inner` (generated).

- **`lib.rs` & `AppContext`**: Core modules (`action`, `api`, `command`, `config`, `context`,
  `cookies`, `db`, `downloader`, `entity`, `error`, `id_parse`, `migration`, `muxer`, `payload`,
  `picker`, `reconcile`, `response`, `state`, `table`, `version`, `wbi`) are exported from
  `fav_bili/src/lib.rs`. `AppContext` (`context.rs`) injects `Db`, `Config`/`Workspace`, HTTP client
  config, and `Arc<dyn Muxer>` instead of relying on global singletons or CWD state. `main.rs`
  returns `std::process::ExitCode` from `AppError`/`ExitKind` (`0` ok, `1` runtime, `2` usage, `3`
  auth, `4` partial failure) and prints `{:#}` chains to `stderr`.
- **`command.rs`**: Builds the clap tree with `Command::new("fav")` (matching `[[bin]] name = "fav"`
  in `fav_bili/Cargo.toml`), global flags (`-C/--dir`, `-v`/`-q`, `--no-progress`,
  `--non-interactive`, `--color`), untracked `fav download` (alias `dl`, dispatched directly without
  opening a read-write `AppContext` or workspace lock), clarity/codec/audio-only/recheck flags, and
  shell completions via `fav completion <shell>` (`CompletionShell`: `bash`, `zsh`, `fish`,
  `powershell`, `elvish`, `nushell`/`nu`, with `--bin-name`, `--static`) backed by the hidden
  `fav __complete <sets|ups|accounts|medias> [prefix]` subcommand with `-C/--dir` forwarding.
  Tracing logs write to `stderr` with a crate-scoped default (`fav_bili=...`) that respects
  `RUST_LOG`.
- **`downloader.rs` & `muxer.rs`**: `downloader.rs` is the shared stateless single-page download/mux
  engine used by both `action/pull.rs` and `action/download.rs` (handling `--sound-only` +
  `--audio-format`, `--clarity`/`--codec`, `try_look=1` for anonymous downloads, on-disk `tmp_dir`
  staging with zero `/tmp` usage, and `(file_size, file_inode)` capture). `muxer.rs` defines the
  `Muxer` trait (`FfmpegMuxer` for runtime, `MockMuxer` for tests). `FfmpegMuxer` spawns
  `tokio::process::Command` with `.kill_on_drop(true)`, `Stdio::null()` stdin, and piped
  `stdout`/`stderr` (bounded stderr capture so `ffmpeg` never corrupts progress bars or JSON stdout).
  Audio-only mode passes `-vn` and supports `copy|m4a|mp3|flac|opus`; all muxed outputs embed
  `-metadata comment=fav:aid=<aid>;cid=<cid>`, readable via `Muxer::read_comment_tag` (`ffprobe`).
  Output is staged to a same-directory `.part` file and atomically renamed on exit `0`; RAII drop
  guards (`PartFileGuard`, `ChildKillGuard`) kill child processes and remove `.part` files on error,
  Ctrl-C/cancellation, or panic.
- **`reconcile.rs`**: Unified 4-layer rename-safe disk-vs-DB reconciliation engine shared by
  `fav doctor`, `fav media reset --missing`, and `fav pull --recheck`: (1) exact `local_path`,
  (2) same-disk `file_inode` auto-relink, (3) recursive `{aid}`/`{bvid}` filename scan (including
  legacy v1.x rows with zero `media_page` entries), and (4) size-matched embedded `fav:aid;cid`
  container comment tag verification for cross-disk renames. Relinks renamed/moved files in place
  and performs surgical per-page resets (`reset_specific_pages`) so intact multi-part pages stay
  `Completed`.
- **`config.rs`**: `Workspace::discover` resolves `-C/--dir` → `FAV_DIR` → upward walk from CWD for
  `.fav/fav.db` → fallback workspace. Merges `~/.config/fav/config.toml` with
  `<workspace>/.fav/config.toml` (including `clarity`/`quality`, `codec`, `sound_only`,
  `audio_format`, `recheck`, `tmp_dir`), acquires `.fav/fav.lock` via `File::try_lock()`
  (`WorkspaceLock`), and enforces Unix `0700`/`0600` permissions on `.fav/` and database files.
- **`db/*`**: `pub struct Db` wraps a `DatabaseConnection` (no global `OnceCell`, no `exit(-1)`).
  `Db::open_or_create` enables `WAL`, `synchronous=NORMAL`, `foreign_keys=true`, `busy_timeout=15s`,
  `sqlx_logging=false`, and runs `Migrator::up`. `Db::open_existing(..., read_only)` skips migrations
  for read-only commands; `Db::open_in_memory()` is used by unit tests. Batch upserts chunk at
  `<= 500` rows inside transactions; `upsert_medias` conflicts on `Column::Id`; `active_media_condition`
  and `pull_candidates` strictly require both an `Active` account and an `Active` set/UP; `prune_all`
  runs in order `sets → medias → ups` inside a transaction while preserving `Completed` medias by
  default.
- **`table.rs`, `id_parse.rs`, `picker.rs`**: `table.rs` handles display-width-aware CJK truncation/padding
  and `OutputFormat` (`table|json|jsonl|tsv|ids`) + `ColorChoice` (`NO_COLOR`). `id_parse.rs` parses
  BVIDs, `av` IDs, raw numeric AIDs, and Bilibili URLs (including `?p=N` page extraction and
  `--page` range specs) with offline BVID↔AID conversion. `picker.rs` implements a TTY-only fuzzy
  multi-select picker on `stderr` (immediately returning `ExitKind::Usage` on non-TTY or
  `--non-interactive`).
- **`action/*`**: Includes `auth`, `sync`, `download`, `status`, `doctor`, `list`, `activate`,
  `deactivate`, `fetch`, `pull`, `media` (`reset`/`skip`), `like`, and `schedule`
  (`systemd`/`launchd`/`task-scheduler`).

### HTTP layer (`api.rs`, `cookies.rs`, `wbi.rs`)

- **Per-account cookie isolation**: Each account uses its own `BiliClient` (`reqwest::Client` +
  dedicated `Arc<reqwest::cookie::Jar>`) — never a process-global cookie jar. `base_url` and
  `passport_base_url` are configurable so integration tests can target local mock HTTP servers.
- **WBI signing**: `WbiEncoder` sorts parameters with `BTreeMap`, strips `!'()*`, RFC-3986
  percent-encodes (`%20`), bounds-checks mixin key length (`>= 64`, no panics), and caches keys in
  `WbiCache` (~6h TTL, invalidated on `-352`/`-403`).
- **Anti-bot & crypto**: `payload/ticket.rs` uses pure-Rust `hmac` + `sha2` (no `ring`).
  `ensure_anti_bot_cookies()` loads account cookies first, keys cookies by name, and bootstraps
  `buvid3`/`bili_ticket`. Credentials use `SecretString` (redacted `Debug`/`Display`).
- **Resilience**: Requests use explicit connect/read timeouts, exponential backoff with jitter on
  429/5xx/`-799`/`-412`/`-352`, nullable array deserialization (`null_as_default`), and streaming
  `.part` downloads with `Range` resume, `backup_url` failover, `Content-Length` verification, and
  optional `--limit-rate`.

### State enums

Enum columns are TEXT; `state.rs` supplies `Display`/`FromStr`/`Serialize`/`Deserialize` and
`Into<sea_orm::Value>` for `AccountState`, `SetState`, `SetKind` (`Created`/`Collected`/`Season`),
`UpState`, `MediaState` (including `Skipped`), `MediaPageState`, and `MediaType` (with
`Unknown(u8)`), plus CLI/config stream enums `VideoClarity`, `VideoCodec`, and `AudioFormat`. Keep
variant lists synchronized with DB queries and migrations.

### Schema changes & migrations

**Migrations are strictly additive-only. NEVER edit existing migrations (`m20250527_000001_create_table.rs`,
`m20261126_000002_enhance_schema.rs`, `m20261127_000003_archive_cursor_and_file_meta.rs`) in place**
— existing user databases have already recorded them as applied and will not re-run them.

To change the schema:
1. Add a new timestamped migration under `fav_bili/src/migration/` (e.g.
   `m20261127_000003_archive_cursor_and_file_meta.rs` added set `kind`/`owner_mid`/`sync_complete`/`resume_page`,
   up `sync_complete`/`resume_page`, and media_page `file_size`/`file_inode`, with one-time
   `sync_complete` backfill) and register it in `fav_bili/src/migration/mod.rs`.
2. Regenerate `fav_bili/src/entity/entity_inner/` with `./sea-orm.sh` (requires `sea-orm-cli`).
   `sea-orm.sh` runs with `set -euo pipefail` against a temporary SQLite DB, generates into a
   same-filesystem staging directory, swaps atomically, and cleans up via `trap` — it does not rely
   on `.env` or leave `data.db` behind. (`.env.example` is provided for manual `sea-orm-cli` use;
   `.env` is gitignored.)
3. Never hand-edit generated files in `entity/entity_inner/` unless maintaining intentional redaction
   hooks (verify `account::Model` continues to redact `cookies` and `refresh_token` in `Debug` after
   regeneration). Hand-written presentation impls live in `entity/mod.rs`.

The `migration` crate exists only to give `sea-orm-cli` a `MigratorTrait` target
(`pub use fav_bili::migration::*`). `default-members = ["fav_bili"]`, so plain `cargo build` skips
it; always pass `--workspace` when linting and testing.

## Runtime behavior worth knowing

- Workspace discovery checks `-C/--dir`, then `FAV_DIR`, then walks upward for `.fav/fav.db`. A
  `.fav/fav.lock` file prevents overlapping runs (e.g., timer + manual sync) from racing.
- **Strict activation invariants**: `fetch`, `pull`, `sync`, and `like` strictly require both an
  `Active` account and an `Active` set/UP. Newly discovered created/collected sets and seasons
  default to `Inactive` (`Unreachable` if remotely invalidated). `fav pull` rejects untracked or
  inactive targets with exit code `2` and points users to `fav activate` or `fav download`.
- **Untracked downloads**: `fav download` (`dl`) never writes SQLite or takes `.fav/fav.lock`,
  defaults to CWD output with `<output_dir>/.fav-tmp/` staging (never `/tmp`), honors `?p=N` in
  URLs unless `--page`/`--all-pages` is passed, and reads active account cookies read-only when
  available (falling back to anonymous `try_look=1`).
- **Incremental cursor fetch**: `fav fetch` persists `sync_complete` and `resume_page` per set/UP
  so already-synced resources stop on the first already-fetched item and interrupted initial scans
  resume cleanly. `--prune` never forces a full scan; only explicit `--full` / `--refresh-status`
  scans all pages and reconciles expired/unfavorited status.
- **Archive-safe pull & reconciliation**: `fav pull` / `fav sync` trust DB `Completed` status by
  default; pass `--recheck` (or run `fav media reset --missing` / `fav doctor`) for 4-layer
  rename-safe reconciliation and surgical per-page recovery.
- `fav sync` combines `fetch` and `pull` in a single command (prefer it over chaining
  `fav fetch && fav pull`).
- Logs and progress bars go to `stderr`; structured output (`--format json|jsonl|tsv|ids`) goes to
  `stdout`.
- Interactive fuzzy pickers activate only on TTYs when no targets are passed; non-TTY invocations
  fail fast with exit code `2`.

## Releasing

Add entries under `## [Unreleased]` in `CHANGELOG.md` (no hard wraps mid-sentence — the file is
rendered as GitHub release notes). Pushing a `v*` tag triggers `.github/workflows/release.yaml`;
crates.io publishing is a manual `workflow_dispatch` via `.github/workflows/publish.yaml`.

Before tagging `vX.Y.Z`, update both `CHANGELOG.md` (`## [X.Y.Z]`) and `fav_bili/Cargo.toml`
(`version = "X.Y.Z"`):
- `release.yaml` verifies `GITHUB_REF_NAME == v<fav_bili/Cargo.toml version>` before creating the
  GitHub Release, builds `fav` with `--locked` for `aarch64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, and `x86_64-pc-windows-msvc` (zero FFmpeg
  build steps), and generates + uploads `SHA256SUMS`.
- `publish.yaml` runs with minimal `permissions: { contents: read }`, passes
  `CARGO_REGISTRY_TOKEN: ${{ secrets.CRATESIO }}` directly to `cargo publish --locked` (no
  `cargo login` disk write), and uses no `eval` in `.github/scripts/`.
