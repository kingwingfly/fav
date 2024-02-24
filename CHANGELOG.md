# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org).

<!--
Note: In this file, do not use the hard wrap in the middle of a sentence for compatibility with GitHub comment style markdown rendering.
-->

## [Unreleased]
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
