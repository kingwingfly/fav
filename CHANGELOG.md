# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org).

<!--
Note: In this file, do not use the hard wrap in the middle of a sentence for compatibility with GitHub comment style markdown rendering.
-->

## [Unreleased]

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
