# fav (`fav_bili`)

A CLI tool to back up your favorite Bilibili collections, followed UPs, and videos.

## Prerequisites & Installation

- **Runtime prerequisite:** [`ffmpeg`](https://ffmpeg.org/) installed and available on `PATH` (or configured via `ffmpeg_path` in `.fav/config.toml`).
- **Zero build-time FFmpeg / `pkgconf` dependency:** `fav` calls the `ffmpeg` CLI subprocess rather than linking `libav*`.

```sh
cargo install fav_bili
```

This installs the `fav` binary. Run `fav doctor` to verify `ffmpeg`, workspace permissions, and account cookies.

## Usage

```sh
Back up your favorite bilibili online resources with CLI.

Usage: fav [OPTIONS] [COMMAND]

Commands:
  init        Initialize a fav workspace in the target directory
  auth        Authenticate and manage Bilibili accounts
  list        List accounts/sets/ups/medias, or show workspace status when run bare [alias: ls, l]
  activate    Activate accounts, favorite sets, or UPs for syncing [alias: active, a]
  deactivate  Deactivate accounts, favorite sets, or UPs [alias: d]
  fetch       Fetch metadata of followed UPs, favorite sets, and medias [alias: f]
  pull        Download and mux pending medias from activated sets/UPs (use `fav download` for untracked videos) [alias: p]
  sync        Fetch metadata and pull pending medias in one command [alias: s]
  download    Download arbitrary Bilibili videos/audio without modifying the workspace DB [alias: dl]
  like        Like (or unlike) Bilibili medias by BVID/AID/URL, set, or stdin
  status      Show workspace summary, media counts, and top UPs
  doctor      Diagnose workspace DB, permissions, ffmpeg, disk sync, and account cookies
  media       Manage tracked media states (reset or skip)
  schedule    Generate (or install) user-level recurring sync schedules
  config      Inspect effective workspace/global configuration and config file paths
  completion  Generate shell completion scripts (with optional dynamic DB completion)
  help        Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose...       Increase log verbosity (-v debug, -vv trace)
  -q, --quiet            Suppress informational logs and next-step hints
  -C, --dir <dir>        Workspace directory containing .fav/ (overrides FAV_DIR)
      --color <color>    Colorize output [default: auto] [possible values: auto, always, never]
      --no-progress      Disable interactive progress bars
      --non-interactive  Disable interactive prompts and TTY pickers
  -h, --help             Print help
  -V, --version          Print version
```

### Quickstart

1. Authenticate via QR code or browser cookies (`--stdin` or `--file`):
   ```sh
   fav auth login --fetch
   # or paste cookies safely via stdin (recommended for `fav like` and remote logout)
   fav auth usecookies --stdin
   ```
2. Activate the favorite sets or UPs you want to back up (interactive TTY fuzzy picker when no IDs are passed):
   ```sh
   fav ls set
   fav activate set
   # or pass IDs directly
   fav activate set 123456789
   ```
3. Sync metadata and download pending videos in one step:
   ```sh
   fav sync
   ```
4. Inspect progress and health:
   ```sh
   fav status
   fav doctor
   ```

### Common Examples

```sh
# Target a workspace directory explicitly (or set FAV_DIR)
fav -C /data/bilibili sync

# Machine-readable output (`table`, `json`, `jsonl`, `tsv`, `ids`)
fav ls media --state pending --format json
fav ls media --set 123456789 --format ids

# Targeted pull, quality/audio selection & disk recovery
fav pull BV17x411w7KC
fav pull --set 123456789 --jobs 4 --limit-rate 5M --clarity 1080p --codec hevc
fav pull --sound-only --audio-format mp3
fav pull --recheck
fav media reset --failed
fav media reset --missing
fav media skip 170001

# Untracked one-off downloads (`fav download` / `fav dl`, supports BVID/URL/`?p=N`/`b23.tv`)
fav dl BV1Q541167Qg
fav dl "https://www.bilibili.com/video/BV1Q541167Qg/?p=2" --sound-only --audio-format flac
fav dl "https://b23.tv/example" --anonymous

# Inspect effective configuration
fav config show
fav config path

# Like medias by set, BVID/URL, or piped IDs
fav like --set 123456789
fav ls media --set 123456789 --format ids | fav like -

# Check cookies
fav auth check -a

# Shell completions (includes built-in dynamic workspace DB completion via `fav __complete`; pass `--static` for static only)
fav completion fish > ~/.config/fish/completions/fav.fish
fav completion bash > ~/.local/share/bash-completion/completions/fav
fav completion zsh > ~/.zfunc/_fav
fav completion powershell | Out-String | Invoke-Expression
fav completion nushell | save -f ~/.config/nushell/fav.nu

# Generate/install periodic sync timers (systemd, launchd, task-scheduler; use --name for multiple workspaces)
fav schedule --target systemd --interval 3h --install
```

### Configuration

`fav` merges `~/.config/fav/config.toml` with `<workspace>/.fav/config.toml` (supporting `output_dir`, `tmp_dir`, `jobs`, `api_concurrency`, `limit_rate`, `clarity`/`quality`, `codec`, `sound_only`, `audio_format`, `recheck`, `prune`, `no_progress`, `color`, and `ffmpeg_path`). Run `fav config show` to inspect effective settings and provenance.

## License

MIT — see [`LICENSE`](https://github.com/kingwingfly/fav/blob/master/LICENSE).
