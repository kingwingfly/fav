# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`fav` is a CLI that backs up bilibili favorites: it authenticates as one or more accounts, fetches
metadata for followed ups / favorite sets / medias into a local SQLite db, then downloads and muxes
the videos. Workspace: `fav_bili` (the whole app) + `migration` (a thin sea-orm-cli shim).

## Build, lint, run

ffmpeg is a hard build dependency — `avmux` dynamically links system `libav*` via pkg-config.

```sh
cargo build
cargo run -- ls set                                  # binary is fav_bili, help text says `fav`
cargo clippy --all-features --all-targets -- -D warnings   # the CI gate (.github/scripts/test.sh)
```

There is **no test suite** — no `#[test]`/`#[cfg(test)]` anywhere, no `tests/` dir. CI's `test` job
only runs `cargo fetch --locked` plus the clippy command above. `-D warnings` means clippy lints are
build failures; there is no `cargo fmt --check` in CI.

`avmux` 0.3 is built on `ffmpeg-next`/`ffmpeg-sys-next`, which support ffmpeg 4 through 9 and probe
the installed libraries themselves, so there are **no ffmpeg version features** — the `ffmpeg6`…
`ffmpeg8` flags from the `avmux` 0.2 (`rsmpeg`) era are gone. To build against a specific install,
set `FFMPEG_DIR` (expects `$FFMPEG_DIR/lib` and `$FFMPEG_DIR/include`) on every platform; the older
`FFMPEG_PKG_CONFIG_PATH`/`FFMPEG_LIBS_DIR`/`FFMPEG_INCLUDE_DIR` names were `rusty_ffmpeg`-only and
are now inert. `avmux`'s own `build`/`static` features are deliberately not re-exported by
`fav_bili`, since CI lints with `--all-features` and `build` would compile ffmpeg from source.

If `build.rs` warns `repository path ... is not owned by current user`, vergen can't read git and
`VERGEN_GIT_DESCRIBE` goes unset, so `--version` reports the tag as `crates.io`.

## Architecture

`main.rs` → `command.rs` → `action/*` → (`payload/*` + `response/*` for HTTP, `db/*` for storage) →
`entity/entity_inner` (generated).

- **`command.rs`** builds the whole clap `Command` tree by hand (no derive) and dispatches with a
  nested `match` on `subcommand()`. New CLI surface means editing both the builder and the match.
- **`action/*`** are free `async fn`s re-exported flat by `action/mod.rs`; each is one CLI verb.
  `activate.rs`/`deactivate.rs` generate their fns with a `paste!` macro over `account, set, up`,
  which must pair with identically-named methods on `Db`.
- **`db/*`** is one `impl Db` block per table over a single `DatabaseConnection`. `db(create)` is a
  process-global `OnceCell`; `db(false)` opens `mode=rw` and `exit(-1)`s with "Login first" when the
  db is missing. `Migrator::up` runs on every connect.
- **`entity/entity_inner/` is generated — never hand-edit.** `entity/mod.rs` adds the hand-written
  `ToTableRecord` impls that feed `table.rs`.

### HTTP layer (api_req)

`api.rs` declares two `ApiCaller`s (`AuthApi` for passport, `BiliApi` for api.bilibili.com) carrying
the shared UA/Referer. To add an endpoint: a `#[derive(Payload, Serialize)]` struct in `payload/`
with `#[api_req(path = …, method = …, req = query|form|json)]`, plus a `Deserialize` struct in
`response/`. The response type is chosen by **inference at the call site** — `ApiCaller::request<P, O>`
is generic, so `let FooResp { .. } = BiliApi::request(FooPayload)` is what binds `O`.

**wbi signing landmine:** `WbiEncoder::encode` md5s the urlencoded serialization of the payload, so
fields in wbi-signed payloads must stay in alphabetical order (see the `// Do not change the field
order` in `payload/set.rs`). Reordering breaks signing silently.

Cookies live in `api_req::COOKIE_JAR`, which is **process-global**. `fetch`/`pull` therefore iterate
accounts *sequentially*, re-seeding the jar per account via `cookies::add_cookie_jar`, and only
parallelize *within* one account (`buffer_unordered`). Don't parallelize the outer loop.

`like` hard-requires `bili_jct` in the stored cookies (it is the csrf token and seeds the ticket
HMAC), which is why the help text tells users to paste browser cookies via `auth usecookies`. It also
bootstraps anti-bot cookies when missing or expired — HMAC-SHA256-derived `bili_ticket`
(`payload/ticket.rs`) and `buvid3` — and writes the refreshed cookie string back to the account row.

### State enums

Enum columns are TEXT, so generated Models type them as `String`. `state.rs`'s `impl_display_fromstr!`
macro supplies `Display`/`FromStr`/`Into<sea_orm::Value>` for `AccountState`, `SetState`, `MediaState`,
`UpState`: query with `Column::State.eq(AccountState::Active)`, store with `.to_string()`. Variant
lists must stay in sync with the `enumeration(...)` arrays in the migration, and with the raw SQL in
`db/media.rs` that hardcodes `state = 'Active'` / `'Pending'`.

### Schema changes

Edit `fav_bili/src/migration/m*_create_table.rs`, then regenerate entities with `./sea-orm.sh`
(needs `sea-orm-cli`). It runs `migrate refresh` against `DATABASE_URL` from `.env` — a dev-only
`data.db`, not the real `.fav/fav.db` — then rewrites `entity/entity_inner/`. The `migration` crate
exists only to give sea-orm-cli a `MigratorTrait` target (`pub use fav_bili::migration::*`), which is
also the reason `fav_bili` has a `lib.rs`. `default-members = ["fav_bili"]`, so plain `cargo build`
skips it.

## Runtime behavior worth knowing

- Everything is **CWD-relative**: the db is `./.fav/fav.db` and downloads land in `./`. A "fav
  directory" is a working directory; commands run elsewhere won't see those accounts.
- `fetch` is run **twice** in the normal flow (`auth` → `fetch` → `activate` → `fetch` → `pull`): the
  first pass only records sets/ups, the second pass enumerates medias of the now-active ones, and
  `pull` downloads only `all_active_pending_medias()`.
- `-v` switches the tracing filter from info to debug, layered on `EnvFilter::from_default_env()`, so
  `RUST_LOG` also applies.
- The released binary is `fav_bili`; users rename it to `fav`, and `completion` reads the name from
  `current_exe()`.

## Releasing

Add entries under `## [Unreleased]` in CHANGELOG.md (no hard wraps mid-sentence — the file is
rendered as GitHub comment markdown). Pushing a `v*` tag drives `release.yaml`, which feeds
CHANGELOG.md to the GitHub release; crates.io publishing is a manual `workflow_dispatch`.

Before tagging `vX.Y.Z`, both **CHANGELOG.md needs a matching `## [X.Y.Z]` section** and
`fav_bili/Cargo.toml` needs `version = "X.Y.Z"`. Nothing enforces the pair:
`create-gh-release-action` hard-fails the whole release when the changelog section is missing, but
never looks at Cargo.toml, so a tag with a stale manifest version releases a binary that misreports
its own `--version`. That already happened at `v1.2.5`, which shipped `version = "1.2.4"`.
