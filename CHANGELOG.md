# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org).

<!--
Note: In this file, do not use the hard wrap in the middle of a sentence for compatibility with GitHub comment style markdown rendering.
-->

## [Unreleased]
## [1.0.5] - 2025-06-02

- `fav like` will check `bili_ticket` cookies or generate one.

## [1.0.5] - 2025-06-01

- `fav auth check` to check cookies usablity.

## [1.0.4] - 2025-06-01

- fix missing alias `c` for `collecion` for `deactivate` sub command

## [1.0.3] - 2025-06-01

- fix a few bugs in fetching

## [1.0.2] - 2025-06-01

- Simplify code with paste macro

## [1.0.1] - 2025-06-01 {1.0.0}

⚠️ fav v0.* is archived [branch](https://github.com/kingwingfly/fav/tree/fav_v0)

`fav v0.*`, based on `fav_core` `fav_utils` (which heavily depends on **protobuf** and many traits),
is considered over-designed.

As my being more familar with Rust, decision has been made to re-factor again this CRUD-oriented application.

🆕 update of fav v1.*

- **sqlite & sea-orm**: to support more media attributes management
- **more powerful**: support pull medias in fav collections and upper space
- **dep:api_req**: my published api request helper crate
- **migrating tool**: [WIP] migrator from `fav v0.*` to `fav v1.*`

## [1.0.0-alpha.*] - 2025-05-31

See [1.0.0 CHANGELOG](#{1.0.0})

## [0.2.39] - 2024-12-23

- tokio runtime: instead of using threads as the number of cpus, now use the single-threaded runtime.

## [0.2.38] - 2024-12-23

- rename `fav daemon` to `fav cron`. And `fav daemon` is still available as an alias.
- bump deps

## [0.2.37] - 2024-11-26

- bump deps

## [0.2.36] - 2024-11-12

- fix: `fav daemon` uses data cached in memory instead of reading again,
leading to repeated pullings with `fav pull` called manually during daemon.

## [0.2.35] - 2024-11-10

- enhance: `fav -d /path` to set working directory.
- enhance: `fav auth reuse` can receive path both containing or not containing `.fav`.
- enhance: add a systemd service example in README.

## [0.2.34] - 2024-11-06

- bump dependencies

## [0.2.33] - 2024-09-05

- fav status will show name of resource creator.
- enhance: Owner trait for resource.

## [0.2.32] - 2024-08-21

- fix: `fav pull <res_id>` will now forcely pull the resource.

## [0.2.31] - 2024-08-18

- improve: `pull` will check ffmpeg before running.

## [0.2.30] - 2024-06-18

- fix: wrong hint of `fav status -h`
- fix: status won't be saved after each epoch of fav daemon

## [0.2.29] - 2024-06-18

- achives(合集) support. Use `fav status -a` to show the achives.

## [0.2.28] - 2024-06-07

- improve: annotation of `fav_derive`
- fix daemon: double SIGINT handlers when pulling

## [0.2.27] - 2024-06-07

- improve follow [this](https://users.rust-lang.org/t/i-just-wrote-the-hardest-code-in-my-life-any-improvements/112596)

## [0.2.26] - 2024-06-07

- yanked

## [0.2.25] - 2024-06-07

- yanked

## [0.2.24] - 2024-06-06

- fix: batch pull SIGINT hint missed

## [0.2.23] - 2024-06-06

- core: refactored, more reliable
- fix: SIGINT handle

## [0.2.22] - 2024-06-06

- tracing: use stdout as tracing output while stderr as processbar

## [0.2.21] - 2024-06-05

- improve progress bar
- skip and contine if error happens while pulling chunks
- terminate if network disconnected
- improve error message if resource inaccessible
- improve error message during daemon
- core: fix a doc test which will create temp dir

## [0.2.20] - 2024-06-05

- better documente for `fav_core`
- fix bugs in batch ops
- fix ctrl-c not handled: handle `SIGINT` in `fav_core`'s Ext methods
- auth: auto fetch after login
- (un)track: show hint after (un)track
- pull: id not found hint improve
- cli version: remove git timestamp
- fix: `batch_pull` and `pull` handle SIGINT together, leading multi info log.
- refactor all commands
- fix: cache not clear if failed or cancelled

## [0.2.19] - 2024-06-05

- yanked

## [0.2.18] - 2024-06-04

- optimize: lto set to `fat` to reduce binary size.
- init: add confirming if '.fav' already exists.

## [0.2.17] - 2024-05-17

- improve: show more infos(timestamp) in `fav -V`.
- fix: meaningless version message when building without a git tree.
- optimize: speed up `fav fetch`

## [0.2.16] - 2024-05-10

- improve: show expired status in `fav status bvid`.
- improve: show more infos(git commit hash and rustc info) in `fav -V`.

## [0.2.15] - 2024-05-06

- utils: remove duplicated status change.
- fix: `fav fetch` will mark all resources as `expired` if no network. This command fetches sets before resources, so the bug only happens when network disconnected after fetching sets successfully, it's rare.

## [0.2.14] - 2024-04-25

- core: FavCoreError will show a message.
- pull: Sometimes, users may collect paid videos, but have no permission to watch full videos. This lead to a `SerdePointerError`. Fix: a message ask users to untrack the video has been added.
- development: `fav_utils_old` will no longer be included into workspace.

## [0.2.13] - 2024-04-24

- bump dependencies
- core: improve log

## [0.2.12] - 2024-04-10

- bump dependencies.
- daemon: show error.
- daemon: performance improvement.

## [0.2.11] - 2024-03-18

- bump dependencies.
- improvement: quit while running `daemon` in a dir which is not initialized.

## [0.2.10] - 2024-03-13

- bump dependencies

## [0.2.9] - 2024-03-05

- fix: modify src/ while building. This does not influence users.

## [0.2.8] - 2024-03-04

- Auth reuse: use hard link instead of copying file
- Stop publish utils and cli to crates.io, because `Cargo` only allow modify `OutDir` during building now.
- Sync the version of cli and bin.

## [0.2.7] - 2024-03-03

- improve error hint of IoErr

## [0.2.6] - 2024-02-29

- Untrack: When untrack a set, it will clear its content now, or `fav status` would keep showing its resources after untracking.

## [0.2.5] - 2024-02-29

- Improvement: the help info of `pull`.
- Auth: Add `reuse` to `fav auth reuse` to reuse the old cookies.

## [0.2.4] - 2024-02-24

- Fix: media count not refresh after fetching.

## [0.2.3] - 2024-02-24

- TryFix: panic when `base_url` not exist.

## [0.2.2] - 2024-02-20

- Fix: overwriting the same file when pulling resources with the same name.
- Improvement: the help info of `pull`.

## [0.2.1] - 2024-02-20

- Handle Expired.
- Pull: If `pull bvid`, `fav` will force to pull it, as long as it's tracked and not expired.

## [0.2.0] - 2024-02-20

- Broken upgrade: the new `fav` is not compatible with the old `fav`. You need to delete `.fav` dir and re-`init` your `fav` after upgrading to `0.2.0`.
- Refactor: `fav` is completely rewritten in rusty style, and is now more generic and more maintainable.
- Simplify: Only `fetch` `pull` `status` `track` `init` `auth` `daemon` `completion` commands are supported now. The `modify` command is removed, since it's too tedious to modify status through a CLI tool.
- Status: Now `status` only show id, title and few status.What's more, use --sets instead of --list, --res instead of --video
- Track: Now `track` does not support resource not in remote favorite sets. (In other words, there's no data only in local, but not in remote.)
- Pull: Now `pull` will call `fetch` first, and resources not tracked and fetched will never able to be pulled.
- Init: Only support bilibili now, so no args needed after `init`.
- Daemon: Now iterval less that 15min will only show a warning, and won't exit.

## [0.1.13] - 2024-02-08

- Fix: `fav completion` generate the wrong script.

## [0.1.12] - 2024-02-07

- Fix: args parsing error when using `fav modify` `fav init` command.

## [0.1.11] - 2024-02-07

- Fix: `Ctrl-C` only cancels current batch(10) of jobs, instead of exiting the whole program.
- See discussions in #5 for more information about the next developping trends.

## [0.1.10] - 2024-02-06

- add `fav completion` command to support auto completion for `bash`, `elvish`, `fish`, `powershell`, `zsh` shell; Run `fav completion -h` for more information. (e.g. run `fav completion fish > ~/.config/fish/completions/fav.fish` to register the auto completion script for `fish`; You can google `where to put completions for xxshell` to find the right place to put the completion script for your shell.)

## [0.1.9] - 2024-02-06

- auto complete support for `zsh` and `fish`; Run `fav complete -h` for more information. (e.g. run `fav complete --shell fish --register ~/.config/fish/completions` to register the auto completion script for `fish`)
- I'll also upload some other auto completion scripts for `bash` and `powershell` and so on.

## [0.1.8] - 2024-02-05

- increased version to 0.1.8
- narrow unsafe closure
- upgrade git action
