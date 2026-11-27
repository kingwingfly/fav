use std::{
    io::{self, Write as _},
    path::PathBuf,
    str::FromStr as _,
    sync::{Arc, OnceLock, RwLock},
};

use anyhow::anyhow;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use clap_complete::Shell;
use console::Term;
use indicatif::MultiProgress;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};

use crate::{
    action::{
        activate::{ActivateOptions, TargetKind, activate},
        auth::{
            CheckOptions, LoginOptions, LogoutOptions, UseCookiesOptions, check, login, logout,
            use_cookies,
        },
        config_cmd::{config_path, config_show},
        deactivate::{DeactivateOptions, deactivate},
        doctor::{DoctorOptions, doctor_with_options},
        download::{DownloadOptions, download},
        fetch::{FetchOptions, fetch},
        like::{LikeOptions, like},
        list::{ListKind, ListOptions, list},
        media::{MediaResetOptions, reset as media_reset, skip as media_skip},
        pull::{PullOptions, pull},
        schedule::{ScheduleTarget, schedule_named},
        status::status,
        sync::{SyncOptions, sync},
    },
    config::{Workspace, parse_rate_limit},
    context::{AppContext, RuntimeOptions},
    db::Db,
    error::{AppError, AppResult},
    state::{AudioFormat, VideoClarity, VideoCodec},
    table::{ColorChoice, OutputFormat, should_color, write_stdout_all, write_stdout_line},
    version::VERSION,
};

const COMPLETION_AFTER_HELP: &str = "\
Examples:
  bash:       fav completion bash > ~/.local/share/bash-completion/completions/fav
  zsh:        fav completion zsh > ~/.zfunc/_fav
  fish:       fav completion fish > ~/.config/fish/completions/fav.fish
  powershell: fav completion powershell | Out-String | Invoke-Expression
  nushell:    fav completion nushell | save -f ~/.config/nushell/fav.nu  # then `source ~/.config/nushell/fav.nu` in config.nu";

/// Target shell for `fav completion <shell>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell", alias = "pwsh")]
    PowerShell,
    Elvish,
    #[value(name = "nushell", alias = "nu")]
    Nushell,
}

/// Custom clap value parser for [`VideoClarity`] that advertises all canonical tier names for
/// static shell completions while still accepting numeric `qn` codes (`80`, `120`, ...) and aliases.
#[derive(Clone)]
struct VideoClarityParser;

impl clap::builder::TypedValueParser for VideoClarityParser {
    type Value = VideoClarity;

    fn parse_ref(
        &self,
        cmd: &Command,
        _arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let raw = value.to_str().ok_or_else(|| {
            let mut c = cmd.clone();
            c.error(
                clap::error::ErrorKind::InvalidUtf8,
                "Invalid UTF-8 for video clarity",
            )
        })?;
        VideoClarity::from_str(raw).map_err(|msg| {
            let mut c = cmd.clone();
            c.error(clap::error::ErrorKind::InvalidValue, msg)
        })
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        Some(Box::new(
            VideoClarity::ALL_NAMES
                .iter()
                .map(|name| clap::builder::PossibleValue::new(*name)),
        ))
    }
}

static ACTIVE_PROGRESS: OnceLock<Arc<RwLock<Option<MultiProgress>>>> = OnceLock::new();

fn progress_slot() -> &'static Arc<RwLock<Option<MultiProgress>>> {
    ACTIVE_PROGRESS.get_or_init(|| Arc::new(RwLock::new(None)))
}

struct ProgressSlotGuard;

impl ProgressSlotGuard {
    fn install(mp: Option<MultiProgress>) -> Self {
        if let Ok(mut guard) = progress_slot().write() {
            *guard = mp;
        }
        Self
    }
}

impl Drop for ProgressSlotGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = progress_slot().write() {
            *guard = None;
        }
    }
}

#[derive(Clone, Copy)]
struct StderrBridgeMakeWriter;

struct StderrBridgeWriter {
    buf: Vec<u8>,
}

impl<'a> MakeWriter<'a> for StderrBridgeMakeWriter {
    type Writer = StderrBridgeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        StderrBridgeWriter {
            buf: Vec::with_capacity(128),
        }
    }
}

impl io::Write for StderrBridgeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let mp = progress_slot().read().ok().and_then(|g| g.clone());
        let res = match mp {
            Some(mp) => mp.suspend(|| {
                let mut err = io::stderr().lock();
                err.write_all(&self.buf)?;
                err.flush()
            }),
            None => {
                let mut err = io::stderr().lock();
                err.write_all(&self.buf)?;
                err.flush()
            }
        };
        self.buf.clear();
        res
    }
}

impl Drop for StderrBridgeWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn init_tracing(runtime: &RuntimeOptions) {
    let crate_name = env!("CARGO_CRATE_NAME");
    let rust_log_set = std::env::var_os("RUST_LOG").is_some_and(|v| !v.is_empty());

    let filter = if runtime.quiet {
        EnvFilter::new(format!("{crate_name}=error"))
    } else if runtime.verbose >= 2 {
        EnvFilter::new(format!("{crate_name}=trace"))
    } else if runtime.verbose == 1 {
        EnvFilter::new(format!("{crate_name}=debug"))
    } else if rust_log_set {
        EnvFilter::from_default_env()
    } else {
        EnvFilter::new(format!("{crate_name}=info"))
    };

    let ansi = should_color(runtime.color, Term::stderr().is_term());
    let builder = tracing_subscriber::fmt()
        .with_writer(StderrBridgeMakeWriter)
        .with_env_filter(filter)
        .with_ansi(ansi);

    if runtime.verbose > 0 {
        let _ = builder
            .with_thread_ids(true)
            .with_line_number(true)
            .try_init();
    } else {
        let _ = builder.with_target(false).without_time().try_init();
    }
}

fn format_args() -> [Arg; 2] {
    [
        Arg::new("format")
            .long("format")
            .help("Output format")
            .value_parser(value_parser!(OutputFormat))
            .default_value("table"),
        Arg::new("json")
            .long("json")
            .help("Shorthand for --format json")
            .action(ArgAction::SetTrue)
            .conflicts_with("format"),
    ]
}

fn resolve_format(matches: &ArgMatches) -> OutputFormat {
    if matches.get_flag("json") {
        OutputFormat::Json
    } else {
        matches
            .get_one::<OutputFormat>("format")
            .copied()
            .unwrap_or(OutputFormat::Table)
    }
}

fn explicit_format(matches: &ArgMatches) -> Option<OutputFormat> {
    if matches.value_source("json") == Some(clap::parser::ValueSource::CommandLine)
        && matches.get_flag("json")
    {
        return Some(OutputFormat::Json);
    }
    if matches.value_source("format") == Some(clap::parser::ValueSource::CommandLine) {
        return matches.get_one::<OutputFormat>("format").copied();
    }
    None
}

fn resolve_subcommand_format(parent: &ArgMatches, child: &ArgMatches) -> AppResult<OutputFormat> {
    match (explicit_format(parent), explicit_format(child)) {
        (Some(_), Some(_)) => Err(AppError::usage(anyhow!(
            "Cannot specify output format flags (--format / --json) both before and after the subcommand."
        ))),
        (Some(fmt), None) | (None, Some(fmt)) => Ok(fmt),
        (None, None) => Ok(resolve_format(child)),
    }
}

fn list_common_args() -> Vec<Arg> {
    let mut args = format_args().to_vec();
    args.extend([
        Arg::new("wide")
            .long("wide")
            .alias("no-truncate")
            .help("Do not truncate long titles/names")
            .action(ArgAction::SetTrue),
        Arg::new("state")
            .long("state")
            .help("Filter by state (comma-separated, e.g. pending,failed)"),
        Arg::new("active")
            .long("active")
            .help("Show only active items")
            .action(ArgAction::SetTrue),
        Arg::new("inactive")
            .long("inactive")
            .help("Show only inactive items")
            .action(ArgAction::SetTrue)
            .conflicts_with("active"),
        Arg::new("search")
            .long("search")
            .help("Case-insensitive substring search"),
        Arg::new("sort")
            .long("sort")
            .help("Sort field (prefix with '-' or suffix ':desc' for descending)"),
        Arg::new("limit")
            .long("limit")
            .help("Maximum number of rows to display")
            .value_parser(value_parser!(u64)),
    ]);
    args
}

fn fetch_flags() -> Vec<Arg> {
    vec![
        Arg::new("sets")
            .long("sets")
            .help("Fetch favorite sets only")
            .action(ArgAction::SetTrue)
            .conflicts_with("ups"),
        Arg::new("ups")
            .long("ups")
            .help("Fetch followed UPs only")
            .action(ArgAction::SetTrue)
            .conflicts_with("sets"),
        Arg::new("set")
            .long("set")
            .help("Limit fetch to specific set ID(s)")
            .value_parser(value_parser!(i64))
            .action(ArgAction::Append),
        Arg::new("up")
            .long("up")
            .help("Limit fetch to specific UP ID(s)")
            .value_parser(value_parser!(i64))
            .action(ArgAction::Append),
        Arg::new("full")
            .long("full")
            .alias("refresh-status")
            .help("Disable incremental cursor early-stop, scan all pages, and refresh expired/unfavorited status")
            .action(ArgAction::SetTrue),
        Arg::new("prune")
            .long("prune")
            .short('p')
            .help("Prune unfaved sets, unfollowed UPs, and inactive non-completed medias")
            .action(ArgAction::SetTrue),
        Arg::new("purge-completed")
            .long("purge-completed")
            .help("Also remove Completed medias when pruning")
            .action(ArgAction::SetTrue),
        Arg::new("dry-run")
            .long("dry-run")
            .help("Preview changes without modifying the database or filesystem")
            .action(ArgAction::SetTrue),
        Arg::new("ignore-errors")
            .long("ignore-errors")
            .help("Exit 0 even if partial errors occurred")
            .action(ArgAction::SetTrue),
    ]
}

fn pull_flags(include_shared_scope: bool) -> Vec<Arg> {
    let mut args = Vec::new();
    if include_shared_scope {
        args.extend([
            Arg::new("set")
                .long("set")
                .help("Pull medias belonging to specific set ID(s)")
                .value_parser(value_parser!(i64))
                .action(ArgAction::Append),
            Arg::new("up")
                .long("up")
                .help("Pull medias published by specific UP ID(s)")
                .value_parser(value_parser!(i64))
                .action(ArgAction::Append),
            Arg::new("dry-run")
                .long("dry-run")
                .help("Show planned downloads without touching network or disk")
                .action(ArgAction::SetTrue),
            Arg::new("ignore-errors")
                .long("ignore-errors")
                .help("Exit 0 even if some downloads fail")
                .action(ArgAction::SetTrue),
        ]);
    }
    args.extend([
        Arg::new("limit")
            .long("limit")
            .help("Maximum number of medias to download")
            .value_parser(value_parser!(usize)),
        Arg::new("jobs")
            .long("jobs")
            .short('j')
            .help("Maximum concurrent downloads (default: 4)")
            .value_parser(value_parser!(usize)),
        Arg::new("output")
            .long("output")
            .short('o')
            .help("Output directory for downloaded media files")
            .value_parser(value_parser!(PathBuf)),
        Arg::new("limit-rate")
            .long("limit-rate")
            .help("Per-stream bandwidth limit (e.g. 5M, 500K)"),
        Arg::new("retry-failed")
            .long("retry-failed")
            .help("Include previously Failed medias")
            .action(ArgAction::SetTrue),
        Arg::new("force")
            .long("force")
            .help("Re-download medias even if already Completed")
            .action(ArgAction::SetTrue),
        Arg::new("sound-only")
            .long("sound-only")
            .alias("audio-only")
            .help("Download audio stream only (skip video stream)")
            .action(ArgAction::SetTrue),
        Arg::new("audio-format")
            .long("audio-format")
            .value_name("FMT")
            .help("Audio output format when using --sound-only (or sound_only = true in config.toml)")
            .value_parser(value_parser!(AudioFormat)),
        Arg::new("clarity")
            .long("clarity")
            .alias("quality")
            .value_name("QUALITY")
            .help("Maximum video clarity/quality (e.g. best, 4k, 1080p60, 1080p, 720p, worst, or numeric qn)")
            .value_parser(VideoClarityParser),
        Arg::new("codec")
            .long("codec")
            .value_name("CODEC")
            .help("Preferred DASH video codec")
            .value_parser(value_parser!(VideoCodec)),
        Arg::new("recheck")
            .long("recheck")
            .help("Recheck disk for missing/renamed Completed files and re-download missing ones")
            .action(ArgAction::SetTrue),
    ]);
    args
}

fn default_schedule_target() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "launchd"
    }
    #[cfg(windows)]
    {
        "task-scheduler"
    }
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        "systemd"
    }
}

/// Build the full clap [`Command`] tree for `fav`.
pub fn build_command() -> Command {
    Command::new("fav")
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .version(VERSION)
        .arg_required_else_help(true)
        .args([
            Arg::new("verbose")
                .help("Increase log verbosity (-v debug, -vv trace)")
                .long("verbose")
                .short('v')
                .global(true)
                .action(ArgAction::Count),
            Arg::new("quiet")
                .help("Suppress informational logs and next-step hints")
                .long("quiet")
                .short('q')
                .global(true)
                .action(ArgAction::SetTrue),
            Arg::new("dir")
                .help("Workspace directory containing .fav/ (overrides FAV_DIR)")
                .long("dir")
                .short('C')
                .global(true)
                .value_parser(value_parser!(PathBuf)),
            Arg::new("color")
                .help("Colorize output")
                .long("color")
                .global(true)
                .value_parser(value_parser!(ColorChoice))
                .default_value("auto"),
            Arg::new("no-progress")
                .help("Disable interactive progress bars")
                .long("no-progress")
                .global(true)
                .action(ArgAction::SetTrue),
            Arg::new("non-interactive")
                .help("Disable interactive prompts and TTY pickers")
                .long("non-interactive")
                .global(true)
                .action(ArgAction::SetTrue),
        ])
        .subcommands([
            Command::new("init").about("Initialize a fav workspace in the target directory"),
            Command::new("auth")
                .about("Authenticate and manage Bilibili accounts")
                .arg_required_else_help(true)
                .subcommands([
                    Command::new("login")
                        .about("Login with a Bilibili QR code")
                        .args([
                            Arg::new("invert")
                                .long("invert")
                                .help("Invert QR colors (for light terminal themes)")
                                .action(ArgAction::SetTrue),
                            Arg::new("url")
                                .long("url")
                                .alias("show-url")
                                .help("Print the raw QR login URL to stderr")
                                .action(ArgAction::SetTrue),
                            Arg::new("fetch")
                                .long("fetch")
                                .help("Immediately fetch sets and followed UPs after login")
                                .action(ArgAction::SetTrue),
                        ]),
                    Command::new("usecookies")
                        .about("Import account cookies (prefer --stdin or --file over positional args)")
                        .args([
                            Arg::new("cookies")
                                .help("Cookie header string (or '-' to read from stdin; avoid positional secrets in shared shells)")
                                .action(ArgAction::Append),
                            Arg::new("stdin")
                                .long("stdin")
                                .help("Read cookies from standard input")
                                .action(ArgAction::SetTrue),
                            Arg::new("file")
                                .long("file")
                                .help("Read cookies from a file (raw header or Netscape cookies.txt)")
                                .value_parser(value_parser!(PathBuf)),
                            Arg::new("fetch")
                                .long("fetch")
                                .help("Immediately fetch sets and followed UPs after importing cookies")
                                .action(ArgAction::SetTrue),
                        ]),
                    Command::new("logout")
                        .about("Logout and remove accounts")
                        .arg_required_else_help(true)
                        .args([
                            Arg::new("all")
                                .help("Logout all authorized accounts")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("account_id"),
                            Arg::new("account_id")
                                .help("Account ID(s) to logout")
                                .value_parser(value_parser!(i64))
                                .action(ArgAction::Append),
                            Arg::new("local")
                                .long("local")
                                .alias("local-only")
                                .help("Delete account locally without calling Bilibili's remote logout API")
                                .action(ArgAction::SetTrue),
                            Arg::new("force")
                                .long("force")
                                .help("Delete locally even if remote logout fails or bili_jct is missing")
                                .action(ArgAction::SetTrue),
                            Arg::new("yes")
                                .long("yes")
                                .short('y')
                                .help("Skip interactive confirmation when using --all")
                                .action(ArgAction::SetTrue),
                        ]),
                    Command::new("check")
                        .about("Verify stored account cookies (defaults to all accounts)")
                        .args({
                            let mut args = format_args().to_vec();
                            args.extend([
                                Arg::new("all")
                                    .help("Check all accounts (default when no account IDs are given)")
                                    .long("all")
                                    .short('a')
                                    .action(ArgAction::SetTrue)
                                    .conflicts_with("account_id"),
                                Arg::new("account_id")
                                    .help("Account ID(s) to check")
                                    .value_parser(value_parser!(i64))
                                    .action(ArgAction::Append),
                                Arg::new("refresh")
                                    .long("refresh")
                                    .help("Attempt automatic cookie refresh when a refresh_token is stored")
                                    .action(ArgAction::SetTrue),
                                Arg::new("wide")
                                    .long("wide")
                                    .alias("no-truncate")
                                    .help("Do not truncate output columns")
                                    .action(ArgAction::SetTrue),
                            ]);
                            args
                        }),
                ]),
            Command::new("list")
                .about("List accounts/sets/ups/medias, or show workspace status when run bare [alias: ls, l]")
                .aliases(["ls", "l"])
                .args(format_args())
                .subcommands([
                    Command::new("account")
                        .about("List accounts [alias: a]")
                        .aliases(["a"])
                        .args(list_common_args()),
                    Command::new("set")
                        .about("List favorite sets [alias: list, collection, s, l, c]")
                        .aliases(["list", "collection", "s", "l", "c"])
                        .args({
                            let mut args = list_common_args();
                            args.extend([
                                Arg::new("account")
                                    .long("account")
                                    .help("Filter sets belonging to account ID")
                                    .value_parser(value_parser!(i64)),
                                Arg::new("kind")
                                    .long("kind")
                                    .value_name("KIND")
                                    .help("Filter sets by kind (created, collected, season)")
                                    .value_parser(["created", "collected", "season"])
                                    .ignore_case(true),
                            ]);
                            args
                        }),
                    Command::new("up")
                        .about("List UPs [alias: upper, u]")
                        .aliases(["upper", "u"])
                        .args({
                            let mut args = list_common_args();
                            args.extend([
                                Arg::new("account")
                                    .long("account")
                                    .help("Filter UPs followed by account ID")
                                    .value_parser(value_parser!(i64)),
                                Arg::new("followed")
                                    .long("followed")
                                    .help("Show only UPs followed by an active account")
                                    .action(ArgAction::SetTrue),
                            ]);
                            args
                        }),
                    Command::new("media")
                        .about("List medias [alias: video, bv, m, v]")
                        .aliases(["video", "bv", "m", "v"])
                        .args({
                            let mut args = list_common_args();
                            args.extend([
                                Arg::new("set")
                                    .long("set")
                                    .help("Filter medias belonging to set ID")
                                    .value_parser(value_parser!(i64)),
                                Arg::new("up")
                                    .long("up")
                                    .help("Filter medias published by UP ID")
                                    .value_parser(value_parser!(i64)),
                                Arg::new("account")
                                    .long("account")
                                    .help("Filter medias belonging to account ID")
                                    .value_parser(value_parser!(i64)),
                                Arg::new("type")
                                    .long("type")
                                    .help("Filter by media type (video, audio, collection)"),
                            ]);
                            args
                        }),
                ]),
            Command::new("activate")
                .about("Activate accounts, favorite sets, or UPs for syncing [alias: active, a]")
                .arg_required_else_help(true)
                .aliases(["active", "a"])
                .subcommands([
                    Command::new("account")
                        .about("Activate accounts (opens interactive picker on TTY if no targets given) [alias: a]")
                        .aliases(["a"])
                        .args([
                            Arg::new("all")
                                .help("Activate all accounts")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("targets"),
                            Arg::new("targets")
                                .help("Account ID(s) or name substring(s) to activate")
                                .action(ArgAction::Append),
                            Arg::new("fetch")
                                .long("fetch")
                                .help("Fetch metadata immediately after activating")
                                .action(ArgAction::SetTrue),
                        ]),
                    Command::new("set")
                        .about("Activate favorite sets (opens interactive picker on TTY if no targets given) [alias: list, collection, s, l, c]")
                        .aliases(["list", "collection", "s", "l", "c"])
                        .args([
                            Arg::new("all")
                                .help("Activate all sets")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("targets"),
                            Arg::new("targets")
                                .help("Set ID(s) or name substring(s) to activate")
                                .action(ArgAction::Append),
                            Arg::new("fetch")
                                .long("fetch")
                                .help("Immediately fetch medias for the activated set(s)")
                                .action(ArgAction::SetTrue),
                        ]),
                    Command::new("up")
                        .about("Activate UPs (opens interactive picker on TTY if no targets given) [alias: upper, u]")
                        .aliases(["upper", "u"])
                        .args([
                            Arg::new("all")
                                .help("Activate all UPs")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("targets"),
                            Arg::new("targets")
                                .help("UP ID(s) or name substring(s) to activate")
                                .action(ArgAction::Append),
                            Arg::new("fetch")
                                .long("fetch")
                                .help("Immediately fetch videos for the activated UP(s)")
                                .action(ArgAction::SetTrue),
                        ]),
                    Command::new("all")
                        .about("Activate all accounts, sets, and UPs")
                        .args([Arg::new("fetch")
                            .long("fetch")
                            .help("Fetch metadata immediately after activating")
                            .action(ArgAction::SetTrue)]),
                ]),
            Command::new("deactivate")
                .about("Deactivate accounts, favorite sets, or UPs [alias: d]")
                .aliases(["d", "deactive"])
                .arg_required_else_help(true)
                .subcommands([
                    Command::new("account")
                        .about("Deactivate accounts [alias: a]")
                        .aliases(["a"])
                        .args([
                            Arg::new("all")
                                .help("Deactivate all authorized accounts")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("targets"),
                            Arg::new("targets")
                                .help("Account ID(s) or name substring(s) to deactivate")
                                .action(ArgAction::Append),
                        ]),
                    Command::new("set")
                        .about("Deactivate favorite sets [alias: list, collection, s, l, c]")
                        .aliases(["list", "collection", "s", "l", "c"])
                        .args([
                            Arg::new("all")
                                .help("Deactivate all sets")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("targets"),
                            Arg::new("targets")
                                .help("Set ID(s) or name substring(s) to deactivate")
                                .action(ArgAction::Append),
                        ]),
                    Command::new("up")
                        .about("Deactivate UPs [alias: upper, u]")
                        .aliases(["upper", "u"])
                        .args([
                            Arg::new("all")
                                .help("Deactivate all UPs")
                                .long("all")
                                .short('a')
                                .action(ArgAction::SetTrue)
                                .conflicts_with("targets"),
                            Arg::new("targets")
                                .help("UP ID(s) or name substring(s) to deactivate")
                                .action(ArgAction::Append),
                        ]),
                    Command::new("all").about("Deactivate all accounts, sets, and UPs"),
                ]),
            Command::new("fetch")
                .about("Fetch metadata of followed UPs, favorite sets, and medias [alias: f]")
                .aliases(["f"])
                .args(fetch_flags()),
            Command::new("pull")
                .about("Download and mux pending medias from activated sets/UPs (use `fav download` for untracked videos) [alias: p]")
                .aliases(["p"])
                .args({
                    let mut args = vec![
                        Arg::new("targets")
                            .help("Specific tracked & active BVID(s), avID(s), numeric AID(s), or URL(s) to pull (use `fav download` for untracked videos)")
                            .action(ArgAction::Append),
                    ];
                    args.extend(pull_flags(true));
                    args
                }),
            Command::new("sync")
                .about("Fetch metadata and pull pending medias in one command [alias: s]")
                .aliases(["s"])
                .args({
                    let mut args = fetch_flags();
                    args.extend(pull_flags(false));
                    args
                }),
            Command::new("download")
                .about("Download arbitrary Bilibili videos/audio without modifying the workspace DB [alias: dl]")
                .aliases(["dl"])
                .arg_required_else_help(true)
                .args([
                    Arg::new("targets")
                        .help("BVID(s), avID(s), numeric AID(s), Bilibili/b23.tv URL(s), or '-' to read from stdin")
                        .action(ArgAction::Append),
                    Arg::new("stdin")
                        .long("stdin")
                        .help("Read target IDs/URLs from standard input (one per line)")
                        .action(ArgAction::SetTrue),
                    Arg::new("page")
                        .long("page")
                        .value_name("SPEC")
                        .help("Download specific page(s) of multi-part videos (e.g. 1, 2-4, 1,3)")
                        .conflicts_with("all-pages"),
                    Arg::new("all-pages")
                        .long("all-pages")
                        .help("Download all pages even if the target URL specifies ?p=N")
                        .action(ArgAction::SetTrue),
                    Arg::new("sound-only")
                        .long("sound-only")
                        .alias("audio-only")
                        .help("Download audio stream only (skip video stream)")
                        .action(ArgAction::SetTrue),
                    Arg::new("audio-format")
                        .long("audio-format")
                        .value_name("FMT")
                        .help("Audio output format when using --sound-only (or sound_only = true in config.toml)")
                        .value_parser(value_parser!(AudioFormat)),
                    Arg::new("clarity")
                        .long("clarity")
                        .alias("quality")
                        .value_name("QUALITY")
                        .help("Maximum video clarity/quality (e.g. best, 4k, 1080p60, 1080p, 720p, worst, or numeric qn)")
                        .value_parser(VideoClarityParser),
                    Arg::new("codec")
                        .long("codec")
                        .value_name("CODEC")
                        .help("Preferred DASH video codec")
                        .value_parser(value_parser!(VideoCodec)),
                    Arg::new("output")
                        .long("output")
                        .short('o')
                        .value_name("DIR")
                        .help("Output directory (defaults to current working directory)")
                        .value_parser(value_parser!(PathBuf)),
                    Arg::new("jobs")
                        .long("jobs")
                        .short('j')
                        .value_name("N")
                        .help("Maximum concurrent downloads (default: 4)")
                        .value_parser(value_parser!(usize)),
                    Arg::new("limit-rate")
                        .long("limit-rate")
                        .value_name("RATE")
                        .help("Per-stream bandwidth limit (e.g. 5M, 500K)"),
                    Arg::new("force")
                        .long("force")
                        .help("Overwrite existing output files")
                        .action(ArgAction::SetTrue),
                    Arg::new("anonymous")
                        .long("anonymous")
                        .help("Download anonymously without reading workspace account cookies")
                        .action(ArgAction::SetTrue),
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Preview planned downloads without downloading files")
                        .action(ArgAction::SetTrue),
                    Arg::new("ignore-errors")
                        .long("ignore-errors")
                        .help("Exit 0 even if some downloads fail")
                        .action(ArgAction::SetTrue),
                ]),
            Command::new("like")
                .about("Like (or unlike) Bilibili medias by BVID/AID/URL, set, or stdin")
                .args([
                    Arg::new("targets")
                        .help("BVID(s), avID(s), numeric AID(s), Bilibili/b23.tv URL(s), or '-' to read from stdin")
                        .action(ArgAction::Append),
                    Arg::new("stdin")
                        .long("stdin")
                        .help("Read target IDs/URLs from standard input (one per line)")
                        .action(ArgAction::SetTrue),
                    Arg::new("set")
                        .long("set")
                        .help("Target all medias in specific set ID(s)")
                        .value_parser(value_parser!(i64))
                        .action(ArgAction::Append),
                    Arg::new("all-active")
                        .long("all-active")
                        .help("Target all medias across active sets and UPs")
                        .action(ArgAction::SetTrue),
                    Arg::new("unlike")
                        .long("unlike")
                        .help("Cancel likes instead of liking")
                        .action(ArgAction::SetTrue),
                    Arg::new("account")
                        .long("account")
                        .help("Restrict to specific account ID(s)")
                        .value_parser(value_parser!(i64))
                        .action(ArgAction::Append),
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Preview targets without sending like requests")
                        .action(ArgAction::SetTrue),
                    Arg::new("ignore-errors")
                        .long("ignore-errors")
                        .help("Exit 0 even if some like requests fail")
                        .action(ArgAction::SetTrue),
                ]),
            Command::new("status")
                .about("Show workspace summary, media counts, and top UPs")
                .args({
                    let mut args = format_args().to_vec();
                    args.push(
                        Arg::new("top")
                            .long("top")
                            .help("Number of top UPs to display")
                            .value_parser(value_parser!(usize))
                            .default_value("10"),
                    );
                    args
                }),
            Command::new("doctor")
                .about("Diagnose workspace DB, permissions, ffmpeg, disk sync, and account cookies")
                .args({
                    let mut args = format_args().to_vec();
                    args.extend([
                        Arg::new("offline")
                            .long("offline")
                            .help("Skip live network authentication checks")
                            .action(ArgAction::SetTrue),
                        Arg::new("output")
                            .long("output")
                            .short('o')
                            .value_name("DIR")
                            .help("Output directory to inspect for disk sync checks")
                            .value_parser(value_parser!(PathBuf)),
                    ]);
                    args
                }),
            Command::new("media")
                .about("Manage tracked media states (reset or skip)")
                .arg_required_else_help(true)
                .subcommands([
                    Command::new("reset")
                        .about("Reset medias back to Pending for re-downloading")
                        .args([
                            Arg::new("targets")
                                .help("Specific BVID(s), avID(s), numeric AID(s), or URL(s)")
                                .action(ArgAction::Append),
                            Arg::new("failed")
                                .long("failed")
                                .help("Reset all Failed medias")
                                .action(ArgAction::SetTrue),
                            Arg::new("expired")
                                .long("expired")
                                .help("Reset all Expired medias")
                                .action(ArgAction::SetTrue),
                            Arg::new("permission-denied")
                                .long("permission-denied")
                                .help("Reset all PermissionDenied medias")
                                .action(ArgAction::SetTrue),
                            Arg::new("skipped")
                                .long("skipped")
                                .help("Reset all Skipped medias")
                                .action(ArgAction::SetTrue),
                            Arg::new("completed")
                                .long("completed")
                                .help("Reset all Completed medias")
                                .action(ArgAction::SetTrue),
                            Arg::new("missing")
                                .long("missing")
                                .help("Reset active Completed medias whose downloaded files are missing on disk")
                                .action(ArgAction::SetTrue),
                            Arg::new("include-inactive")
                                .long("include-inactive")
                                .help("Include medias in inactive sets/UPs when scanning with --missing")
                                .action(ArgAction::SetTrue),
                            Arg::new("output")
                                .long("output")
                                .short('o')
                                .value_name("DIR")
                                .help("Output directory to inspect when using --missing")
                                .value_parser(value_parser!(PathBuf)),
                            Arg::new("all")
                                .long("all")
                                .short('a')
                                .help("Reset all tracked medias")
                                .action(ArgAction::SetTrue),
                            Arg::new("dry-run")
                                .long("dry-run")
                                .help("Preview how many medias would be reset")
                                .action(ArgAction::SetTrue),
                            Arg::new("yes")
                                .long("yes")
                                .short('y')
                                .help("Skip interactive confirmation when using --all or --completed")
                                .action(ArgAction::SetTrue),
                        ]),
                    Command::new("skip")
                        .about("Mark medias as Skipped so `fav pull` ignores them")
                        .arg_required_else_help(true)
                        .args([Arg::new("targets")
                            .help("BVID(s), avID(s), numeric AID(s), or URL(s) to skip")
                            .required(true)
                            .action(ArgAction::Append)]),
                ]),
            Command::new("schedule")
                .about("Generate (or install) user-level recurring sync schedules")
                .args([
                    Arg::new("target")
                        .long("target")
                        .help("Scheduler backend")
                        .value_parser(value_parser!(ScheduleTarget))
                        .default_value(default_schedule_target()),
                    Arg::new("interval")
                        .long("interval")
                        .help("Sync interval (e.g. 30m, 3h, 1d)")
                        .default_value("3h"),
                    Arg::new("name")
                        .long("name")
                        .value_name("NAME")
                        .help("Optional suffix for unit/task names to manage multiple workspaces"),
                    Arg::new("bin")
                        .long("bin")
                        .help("Path to the `fav` binary to invoke")
                        .value_parser(value_parser!(PathBuf)),
                    Arg::new("install")
                        .long("install")
                        .help("Write user-level schedule files directly (no sudo)")
                        .action(ArgAction::SetTrue),
                ]),
            Command::new("config")
                .about("Inspect effective configuration and config file paths (bare `fav config` runs `show`)")
                .args(format_args())
                .subcommands([
                    Command::new("show")
                        .about("Show effective configuration values and their provenance")
                        .args(format_args()),
                    Command::new("path")
                        .about("Show global and workspace config file paths")
                        .args(format_args()),
                ]),
            Command::new("completion")
                .about("Generate shell completion scripts (with optional dynamic DB completion)")
                .after_help(COMPLETION_AFTER_HELP)
                .arg_required_else_help(true)
                .args([
                    Arg::new("shell")
                        .help("Target shell")
                        .required(true)
                        .value_parser(value_parser!(CompletionShell)),
                    Arg::new("bin-name")
                        .long("bin-name")
                        .help("Binary name to bind completions to")
                        .default_value("fav"),
                    Arg::new("static")
                        .long("static")
                        .help("Emit only static completions without dynamic DB candidate hooks")
                        .action(ArgAction::SetTrue),
                ]),
            Command::new("__complete")
                .hide(true)
                .about("Internal helper for dynamic shell completions")
                .args([
                    Arg::new("kind")
                        .required(true)
                        .value_parser(["sets", "ups", "accounts", "medias"]),
                    Arg::new("prefix").default_value(""),
                ]),
        ])
}

fn runtime_from_matches(matches: &ArgMatches) -> RuntimeOptions {
    RuntimeOptions {
        explicit_dir: matches.get_one::<PathBuf>("dir").cloned(),
        verbose: matches.get_count("verbose"),
        quiet: matches.get_flag("quiet"),
        color: matches
            .get_one::<ColorChoice>("color")
            .copied()
            .unwrap_or(ColorChoice::Auto),
        color_explicit: matches.value_source("color")
            == Some(clap::parser::ValueSource::CommandLine),
        no_progress: matches.get_flag("no-progress"),
        non_interactive: matches.get_flag("non-interactive"),
    }
}

fn collect_strings(matches: &ArgMatches, name: &str) -> Vec<String> {
    matches
        .get_many::<String>(name)
        .map(|vals| vals.cloned().collect())
        .unwrap_or_default()
}

fn collect_i64s(matches: &ArgMatches, name: &str) -> Vec<i64> {
    matches
        .get_many::<i64>(name)
        .map(|vals| vals.copied().collect())
        .unwrap_or_default()
}

fn parse_list_options(matches: &ArgMatches) -> ListOptions {
    ListOptions {
        format: resolve_format(matches),
        wide: matches.get_flag("wide"),
        state: matches.get_one::<String>("state").cloned(),
        kind: matches
            .try_get_one::<String>("kind")
            .ok()
            .flatten()
            .cloned(),
        set_id: matches.try_get_one::<i64>("set").ok().flatten().copied(),
        up_id: matches.try_get_one::<i64>("up").ok().flatten().copied(),
        account_id: matches
            .try_get_one::<i64>("account")
            .ok()
            .flatten()
            .copied(),
        active_only: matches.get_flag("active"),
        inactive_only: matches.get_flag("inactive"),
        followed_only: matches
            .try_get_one::<bool>("followed")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
        media_type: matches
            .try_get_one::<String>("type")
            .ok()
            .flatten()
            .cloned(),
        search: matches.get_one::<String>("search").cloned(),
        sort: matches.get_one::<String>("sort").cloned(),
        limit: matches.get_one::<u64>("limit").copied(),
    }
}

fn parse_fetch_options(matches: &ArgMatches) -> FetchOptions {
    FetchOptions {
        sets_only: matches.get_flag("sets"),
        ups_only: matches.get_flag("ups"),
        set_ids: collect_i64s(matches, "set"),
        up_ids: collect_i64s(matches, "up"),
        full: matches.get_flag("full"),
        prune: matches.get_flag("prune"),
        purge_completed: matches.get_flag("purge-completed"),
        dry_run: matches.get_flag("dry-run"),
        ignore_errors: matches.get_flag("ignore-errors"),
    }
}

fn parse_pull_options(matches: &ArgMatches, include_targets: bool) -> AppResult<PullOptions> {
    let limit_rate = match matches.get_one::<String>("limit-rate") {
        Some(raw) => Some(parse_rate_limit(raw)?),
        None => None,
    };
    Ok(PullOptions {
        targets: if include_targets {
            collect_strings(matches, "targets")
        } else {
            Vec::new()
        },
        set_ids: collect_i64s(matches, "set"),
        up_ids: collect_i64s(matches, "up"),
        limit: matches.get_one::<usize>("limit").copied(),
        jobs: matches.get_one::<usize>("jobs").copied(),
        output_dir: matches.get_one::<PathBuf>("output").cloned(),
        limit_rate,
        retry_failed: matches.get_flag("retry-failed"),
        force: matches.get_flag("force"),
        sound_only: matches.get_flag("sound-only"),
        audio_format: matches.get_one::<AudioFormat>("audio-format").copied(),
        clarity: matches.get_one::<VideoClarity>("clarity").copied(),
        codec: matches.get_one::<VideoCodec>("codec").copied(),
        recheck: matches.get_flag("recheck"),
        dry_run: matches.get_flag("dry-run"),
        ignore_errors: matches.get_flag("ignore-errors"),
    })
}

fn parse_download_options(matches: &ArgMatches) -> AppResult<DownloadOptions> {
    let limit_rate = match matches.get_one::<String>("limit-rate") {
        Some(raw) => Some(parse_rate_limit(raw)?),
        None => None,
    };
    let raw_targets = collect_strings(matches, "targets");
    let mut read_stdin = matches.get_flag("stdin");
    let mut targets = Vec::with_capacity(raw_targets.len());
    for t in raw_targets {
        if t == "-" {
            read_stdin = true;
        } else {
            targets.push(t);
        }
    }
    Ok(DownloadOptions {
        targets,
        read_stdin,
        pages: matches.get_one::<String>("page").cloned(),
        all_pages: matches.get_flag("all-pages"),
        output_dir: matches.get_one::<PathBuf>("output").cloned(),
        jobs: matches.get_one::<usize>("jobs").copied(),
        limit_rate,
        sound_only: matches.get_flag("sound-only"),
        audio_format: matches.get_one::<AudioFormat>("audio-format").copied(),
        clarity: matches.get_one::<VideoClarity>("clarity").copied(),
        codec: matches.get_one::<VideoCodec>("codec").copied(),
        force: matches.get_flag("force"),
        anonymous: matches.get_flag("anonymous"),
        dry_run: matches.get_flag("dry-run"),
        ignore_errors: matches.get_flag("ignore-errors"),
    })
}

fn sanitize_shell_fn_ident(bin_name: &str) -> String {
    let mut out: String = bin_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    out
}

const VALUE_TAKING_FLAGS_SH: &str = "-C|--dir|--color|--format|--state|--search|--sort|--limit|--type|--kind|--set|--up|--account|-j|--jobs|-o|--output|--limit-rate|--clarity|--quality|--codec|--audio-format|--page|--top|--target|--interval|--name|--bin|--bin-name|--file";

const VALUE_TAKING_FLAGS_FISH: &str = "-C --dir --color --format --state --search --sort --limit --type --kind --set --up --account -j --jobs -o --output --limit-rate --clarity --quality --codec --audio-format --page --top --target --interval --name --bin --bin-name --file";

fn dynamic_completion_snippet(shell: CompletionShell, bin_name: &str) -> String {
    let fn_ident = sanitize_shell_fn_ident(bin_name);
    match shell {
        CompletionShell::Fish => format!(
            "\n# Dynamic workspace DB completions via `{bin_name} __complete`\n\
             function __fish_{fn_ident}_complete\n\
                 set -l kind $argv[1]\n\
                 set -l tokens (commandline -opc)\n\
                 set -l dir_args\n\
                 set -l i 2\n\
                 set -l n (count $tokens)\n\
                 while test $i -le $n\n\
                     switch $tokens[$i]\n\
                         case -C --dir\n\
                             if test (math $i + 1) -le $n\n\
                                 set dir_args -C $tokens[(math $i + 1)]\n\
                             end\n\
                             set i (math $i + 2)\n\
                             continue\n\
                         case '--dir=*'\n\
                             set dir_args -C (string replace -r '^--dir=' '' -- $tokens[$i])\n\
                     end\n\
                     set i (math $i + 1)\n\
                 end\n\
                 {bin_name} $dir_args __complete $kind (commandline -ct) 2>/dev/null\n\
             end\n\
             \n\
             function __fish_{fn_ident}_subcmd_is\n\
                 set -l expected $argv[1]\n\
                 set -l cur (commandline -ct)\n\
                 if string match -q -- '-*' \"$cur\"\n\
                     return 1\n\
                 end\n\
                 set -l tokens (commandline -opc)\n\
                 set -l n (count $tokens)\n\
                 if test $n -lt 2\n\
                     return 1\n\
                 end\n\
                 switch $tokens[-1]\n\
                     case {VALUE_TAKING_FLAGS_FISH}\n\
                         return 1\n\
                 end\n\
                 set -l sub1 \"\"\n\
                 set -l sub2 \"\"\n\
                 set -l i 2\n\
                 while test $i -le $n\n\
                     set -l w $tokens[$i]\n\
                     switch $w\n\
                         case {VALUE_TAKING_FLAGS_FISH}\n\
                             set i (math $i + 2)\n\
                             continue\n\
                         case '-*'\n\
                             set i (math $i + 1)\n\
                             continue\n\
                     end\n\
                     if test -z \"$sub1\"\n\
                         set sub1 $w\n\
                     else if test -z \"$sub2\"\n\
                         set sub2 $w\n\
                     end\n\
                     set i (math $i + 1)\n\
                 end\n\
                 switch $expected\n\
                     case sets\n\
                         switch $sub1\n\
                             case activate active a deactivate deactive d\n\
                                 switch $sub2\n\
                                     case set s list collection l c\n\
                                         return 0\n\
                                 end\n\
                         end\n\
                     case ups\n\
                         switch $sub1\n\
                             case activate active a deactivate deactive d\n\
                                 switch $sub2\n\
                                     case up upper u\n\
                                         return 0\n\
                                 end\n\
                         end\n\
                     case accounts\n\
                         switch $sub1\n\
                             case activate active a deactivate deactive d\n\
                                 switch $sub2\n\
                                     case account a\n\
                                         return 0\n\
                                 end\n\
                             case auth\n\
                                 switch $sub2\n\
                                     case logout check\n\
                                         return 0\n\
                                 end\n\
                         end\n\
                     case medias\n\
                         switch $sub1\n\
                             case pull p like\n\
                                 return 0\n\
                             case media\n\
                                 switch $sub2\n\
                                     case reset skip\n\
                                         return 0\n\
                                 end\n\
                         end\n\
                 end\n\
                 return 1\n\
             end\n\
             \n\
             complete -c {bin_name} -n '__fish_{fn_ident}_subcmd_is sets' -f -a '(__fish_{fn_ident}_complete sets)'\n\
             complete -c {bin_name} -n '__fish_{fn_ident}_subcmd_is ups' -f -a '(__fish_{fn_ident}_complete ups)'\n\
             complete -c {bin_name} -n '__fish_{fn_ident}_subcmd_is accounts' -f -a '(__fish_{fn_ident}_complete accounts)'\n\
             complete -c {bin_name} -n '__fish_{fn_ident}_subcmd_is medias' -f -a '(__fish_{fn_ident}_complete medias)'\n\
             complete -c {bin_name} -l set -r -f -a '(__fish_{fn_ident}_complete sets)'\n\
             complete -c {bin_name} -l up -r -f -a '(__fish_{fn_ident}_complete ups)'\n\
             complete -c {bin_name} -l account -r -f -a '(__fish_{fn_ident}_complete accounts)'\n"
        ),
        CompletionShell::Bash => format!(
            "\n# Dynamic workspace DB completions via `{bin_name} __complete`\n\
             _{fn_ident}_dynamic_candidates() {{\n\
                 local kind=\"$1\" cur=\"$2\"\n\
                 shift 2\n\
                 COMPREPLY=( $(compgen -W \"$({bin_name} \"$@\" __complete \"$kind\" \"$cur\" 2>/dev/null | cut -f1)\" -- \"$cur\") )\n\
             }}\n\
             \n\
             _{fn_ident}_with_dynamic() {{\n\
                 local cur prev\n\
                 cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n\
                 prev=\"${{COMP_WORDS[COMP_CWORD-1]}}\"\n\
                 if [[ \"${{cur}}\" == -* ]]; then\n\
                     _{fn_ident} \"$@\"\n\
                     return $?\n\
                 fi\n\
                 local -a dir_args=()\n\
                 local k=1 kw\n\
                 while [[ $k -lt ${{#COMP_WORDS[@]}} ]]; do\n\
                     kw=\"${{COMP_WORDS[$k]}}\"\n\
                     if [[ $k -ne $COMP_CWORD ]]; then\n\
                         case \"$kw\" in\n\
                             -C|--dir)\n\
                                 if (( k + 1 < ${{#COMP_WORDS[@]}} )) && [[ $((k + 1)) -ne $COMP_CWORD ]]; then\n\
                                     dir_args=(-C \"${{COMP_WORDS[$((k + 1))]}}\")\n\
                                 fi\n\
                                 ((k += 2))\n\
                                 continue\n\
                                 ;;\n\
                             --dir=*)\n\
                                 dir_args=(-C \"${{kw#--dir=}}\")\n\
                                 ;;\n\
                         esac\n\
                     fi\n\
                     ((k += 1))\n\
                 done\n\
                 case \"${{prev}}\" in\n\
                     --set)\n\
                         _{fn_ident}_dynamic_candidates sets \"${{cur}}\" ${{dir_args[@]+\"${{dir_args[@]}}\"}}\n\
                         return 0\n\
                         ;;\n\
                     --up)\n\
                         _{fn_ident}_dynamic_candidates ups \"${{cur}}\" ${{dir_args[@]+\"${{dir_args[@]}}\"}}\n\
                         return 0\n\
                         ;;\n\
                     --account)\n\
                         _{fn_ident}_dynamic_candidates accounts \"${{cur}}\" ${{dir_args[@]+\"${{dir_args[@]}}\"}}\n\
                         return 0\n\
                         ;;\n\
                     {VALUE_TAKING_FLAGS_SH})\n\
                         _{fn_ident} \"$@\"\n\
                         return $?\n\
                         ;;\n\
                 esac\n\
                 local sub1=\"\" sub2=\"\" i=1 w\n\
                 while [[ $i -lt $COMP_CWORD ]]; do\n\
                     w=\"${{COMP_WORDS[$i]}}\"\n\
                     case \"$w\" in\n\
                         {VALUE_TAKING_FLAGS_SH})\n\
                             ((i += 2))\n\
                             continue\n\
                             ;;\n\
                         -*)\n\
                             ((i += 1))\n\
                             continue\n\
                             ;;\n\
                     esac\n\
                     if [[ -z \"$sub1\" ]]; then\n\
                         sub1=\"$w\"\n\
                     elif [[ -z \"$sub2\" ]]; then\n\
                         sub2=\"$w\"\n\
                     fi\n\
                     ((i += 1))\n\
                 done\n\
                 case \"${{sub1}}:${{sub2}}\" in\n\
                     activate:set|activate:s|activate:list|activate:collection|activate:l|activate:c|\\\n\
                     active:set|active:s|active:list|active:collection|active:l|active:c|\\\n\
                     a:set|a:s|a:list|a:collection|a:l|a:c|\\\n\
                     deactivate:set|deactivate:s|deactivate:list|deactivate:collection|deactivate:l|deactivate:c|\\\n\
                     deactive:set|deactive:s|deactive:list|deactive:collection|deactive:l|deactive:c|\\\n\
                     d:set|d:s|d:list|d:collection|d:l|d:c)\n\
                         _{fn_ident}_dynamic_candidates sets \"${{cur}}\" ${{dir_args[@]+\"${{dir_args[@]}}\"}}\n\
                         return 0\n\
                         ;;\n\
                     activate:up|activate:upper|activate:u|active:up|active:upper|active:u|a:up|a:upper|a:u|\\\n\
                     deactivate:up|deactivate:upper|deactivate:u|deactive:up|deactive:upper|deactive:u|d:up|d:upper|d:u)\n\
                         _{fn_ident}_dynamic_candidates ups \"${{cur}}\" ${{dir_args[@]+\"${{dir_args[@]}}\"}}\n\
                         return 0\n\
                         ;;\n\
                     activate:account|activate:a|active:account|active:a|a:account|a:a|\\\n\
                     deactivate:account|deactivate:a|deactive:account|deactive:a|d:account|d:a|\\\n\
                     auth:logout|auth:check)\n\
                         _{fn_ident}_dynamic_candidates accounts \"${{cur}}\" ${{dir_args[@]+\"${{dir_args[@]}}\"}}\n\
                         return 0\n\
                         ;;\n\
                     pull:*|p:*|like:*|media:reset|media:skip)\n\
                         _{fn_ident}_dynamic_candidates medias \"${{cur}}\" ${{dir_args[@]+\"${{dir_args[@]}}\"}}\n\
                         return 0\n\
                         ;;\n\
                 esac\n\
                 _{fn_ident} \"$@\"\n\
             }}\n\
             \n\
             if [[ \"${{BASH_VERSINFO[0]}}\" -eq 4 && \"${{BASH_VERSINFO[1]}}\" -ge 4 || \"${{BASH_VERSINFO[0]}}\" -gt 4 ]]; then\n\
                 complete -F _{fn_ident}_with_dynamic -o nosort -o bashdefault -o default {bin_name}\n\
             else\n\
                 complete -F _{fn_ident}_with_dynamic -o bashdefault -o default {bin_name}\n\
             fi\n"
        ),
        CompletionShell::Zsh => format!(
            "\n# Dynamic workspace DB completions via `{bin_name} __complete`\n\
             _{fn_ident}_complete_db() {{\n\
                 local kind=\"$1\" prefix=\"$2\"\n\
                 shift 2\n\
                 local -a entries\n\
                 local line id desc\n\
                 while IFS=$'\\t' read -r id desc; do\n\
                     [[ -n \"$id\" ]] || continue\n\
                     desc=\"${{desc//:/\\\\:}}\"\n\
                     entries+=(\"${{id}}:${{desc}}\")\n\
                 done < <({bin_name} \"$@\" __complete \"$kind\" \"$prefix\" 2>/dev/null)\n\
                 (( ${{#entries[@]}} )) && _describe -t \"$kind\" \"$kind\" entries\n\
             }}\n\
             \n\
             _{fn_ident}_with_dynamic() {{\n\
                 local cur=\"${{words[CURRENT]}}\"\n\
                 local prev=\"\"\n\
                 if (( CURRENT > 1 )); then\n\
                     prev=\"${{words[CURRENT-1]}}\"\n\
                 fi\n\
                 if [[ \"$cur\" == -* ]]; then\n\
                     _{fn_ident} \"$@\"\n\
                     return $?\n\
                 fi\n\
                 local -a dir_args\n\
                 dir_args=()\n\
                 local k=2 kw\n\
                 while (( k <= ${{#words[@]}} )); do\n\
                     kw=\"${{words[k]}}\"\n\
                     if (( k != CURRENT )); then\n\
                         case \"$kw\" in\n\
                             -C|--dir)\n\
                                 if (( k + 1 <= ${{#words[@]}} && k + 1 != CURRENT )); then\n\
                                     dir_args=(-C \"${{words[k+1]}}\")\n\
                                 fi\n\
                                 (( k += 2 ))\n\
                                 continue\n\
                                 ;;\n\
                             --dir=*)\n\
                                 dir_args=(-C \"${{kw#--dir=}}\")\n\
                                 ;;\n\
                         esac\n\
                     fi\n\
                     (( k += 1 ))\n\
                 done\n\
                 case \"$prev\" in\n\
                     --set)\n\
                         _{fn_ident}_complete_db sets \"$cur\" \"${{dir_args[@]}}\"\n\
                         return $?\n\
                         ;;\n\
                     --up)\n\
                         _{fn_ident}_complete_db ups \"$cur\" \"${{dir_args[@]}}\"\n\
                         return $?\n\
                         ;;\n\
                     --account)\n\
                         _{fn_ident}_complete_db accounts \"$cur\" \"${{dir_args[@]}}\"\n\
                         return $?\n\
                         ;;\n\
                     {VALUE_TAKING_FLAGS_SH})\n\
                         _{fn_ident} \"$@\"\n\
                         return $?\n\
                         ;;\n\
                 esac\n\
                 local sub1=\"\" sub2=\"\" i=2 w\n\
                 while (( i < CURRENT )); do\n\
                     w=\"${{words[i]}}\"\n\
                     case \"$w\" in\n\
                         {VALUE_TAKING_FLAGS_SH})\n\
                             (( i += 2 ))\n\
                             continue\n\
                             ;;\n\
                         -*)\n\
                             (( i += 1 ))\n\
                             continue\n\
                             ;;\n\
                     esac\n\
                     if [[ -z \"$sub1\" ]]; then\n\
                         sub1=\"$w\"\n\
                     elif [[ -z \"$sub2\" ]]; then\n\
                         sub2=\"$w\"\n\
                     fi\n\
                     (( i += 1 ))\n\
                 done\n\
                 case \"${{sub1}}:${{sub2}}\" in\n\
                     activate:set|activate:s|activate:list|activate:collection|activate:l|activate:c|\\\n\
                     active:set|active:s|active:list|active:collection|active:l|active:c|\\\n\
                     a:set|a:s|a:list|a:collection|a:l|a:c|\\\n\
                     deactivate:set|deactivate:s|deactivate:list|deactivate:collection|deactivate:l|deactivate:c|\\\n\
                     deactive:set|deactive:s|deactive:list|deactive:collection|deactive:l|deactive:c|\\\n\
                     d:set|d:s|d:list|d:collection|d:l|d:c)\n\
                         _{fn_ident}_complete_db sets \"$cur\" \"${{dir_args[@]}}\"\n\
                         return $?\n\
                         ;;\n\
                     activate:up|activate:upper|activate:u|active:up|active:upper|active:u|a:up|a:upper|a:u|\\\n\
                     deactivate:up|deactivate:upper|deactivate:u|deactive:up|deactive:upper|deactive:u|d:up|d:upper|d:u)\n\
                         _{fn_ident}_complete_db ups \"$cur\" \"${{dir_args[@]}}\"\n\
                         return $?\n\
                         ;;\n\
                     activate:account|activate:a|active:account|active:a|a:account|a:a|\\\n\
                     deactivate:account|deactivate:a|deactive:account|deactive:a|d:account|d:a|\\\n\
                     auth:logout|auth:check)\n\
                         _{fn_ident}_complete_db accounts \"$cur\" \"${{dir_args[@]}}\"\n\
                         return $?\n\
                         ;;\n\
                     pull:*|p:*|like:*|media:reset|media:skip)\n\
                         _{fn_ident}_complete_db medias \"$cur\" \"${{dir_args[@]}}\"\n\
                         return $?\n\
                         ;;\n\
                 esac\n\
                 _{fn_ident} \"$@\"\n\
             }}\n\
             \n\
             if [ \"$funcstack[1]\" = \"_{fn_ident}\" ]; then\n\
                 compdef _{fn_ident}_with_dynamic {bin_name}\n\
                 _{fn_ident}_with_dynamic \"$@\"\n\
             else\n\
                 compdef _{fn_ident}_with_dynamic {bin_name}\n\
             fi\n"
        ),
        CompletionShell::PowerShell | CompletionShell::Nushell | CompletionShell::Elvish => {
            String::new()
        }
    }
}

fn wire_powershell_dynamic_completer(script: &str, bin_name: &str) -> String {
    let fn_ident = sanitize_shell_fn_ident(bin_name);
    let ps_bin = bin_name.replace('\'', "''");
    let static_var = format!("$__fav_static_{fn_ident}");
    let quoted_header =
        format!("Register-ArgumentCompleter -Native -CommandName '{ps_bin}' -ScriptBlock {{");
    let unquoted_header =
        format!("Register-ArgumentCompleter -Native -CommandName {bin_name} -ScriptBlock {{");
    let replacement = format!("{static_var} = {{");

    let (base_script, has_static) = if script.contains(&quoted_header) {
        (script.replacen(&quoted_header, &replacement, 1), true)
    } else if script.contains(&unquoted_header) {
        (script.replacen(&unquoted_header, &replacement, 1), true)
    } else {
        (script.to_owned(), false)
    };

    let fallback_call = if has_static {
        format!("& {static_var} $wordToComplete $commandAst $cursorPosition")
    } else {
        "$null".to_owned()
    };

    let dynamic_block = r#"
# Dynamic workspace DB completions via `__RAW_BIN__ __complete`
Register-ArgumentCompleter -Native -CommandName '__PS_BIN__' -ScriptBlock ({
    param($wordToComplete, $commandAst, $cursorPosition)

    if ($wordToComplete -like '-*') {
        return (__FALLBACK_CALL__)
    }

    $tokens = @()
    for ($idx = 1; $idx -lt $commandAst.CommandElements.Count; $idx++) {
        $el = $commandAst.CommandElements[$idx]
        if ($el.Extent.StartOffset -lt $cursorPosition -and ($wordToComplete -eq '' -or $el.Extent.EndOffset -lt $cursorPosition)) {
            $tokens += $el.Extent.Text.Trim('"', "'")
        }
    }

    $dirArgs = @()
    for ($k = 0; $k -lt $tokens.Count; $k++) {
        $tok = $tokens[$k]
        if ($tok -eq '-C' -or $tok -eq '--dir') {
            if ($k + 1 -lt $tokens.Count) {
                $dirArgs = @('-C', $tokens[$k + 1])
            }
            $k += 1
        } elseif ($tok -like '--dir=*') {
            $dirArgs = @('-C', $tok.Substring(6))
        }
    }

    $prev = if ($tokens.Count -gt 0) { $tokens[$tokens.Count - 1] } else { '' }
    $kind = ''
    $valueOpts = @(
        '-C', '--dir', '--color', '--format', '--state', '--search', '--sort', '--limit',
        '--type', '--kind', '--set', '--up', '--account', '-j', '--jobs', '-o', '--output',
        '--limit-rate', '--clarity', '--quality', '--codec', '--audio-format', '--page',
        '--top', '--target', '--interval', '--name', '--bin', '--bin-name', '--file'
    )

    switch ($prev) {
        '--set'     { $kind = 'sets' }
        '--up'      { $kind = 'ups' }
        '--account' { $kind = 'accounts' }
        default {
            if ($valueOpts -contains $prev) {
                return (__FALLBACK_CALL__)
            }
        }
    }

    if (-not $kind) {
        $sub1 = ''
        $sub2 = ''
        for ($i = 0; $i -lt $tokens.Count; $i++) {
            $w = $tokens[$i]
            if ($valueOpts -contains $w) {
                $i += 1
                continue
            }
            if ($w -like '-*') {
                continue
            }
            if (-not $sub1) {
                $sub1 = $w
            } elseif (-not $sub2) {
                $sub2 = $w
            }
        }

        switch ("$($sub1):$($sub2)") {
            { $_ -in @(
                'activate:set', 'activate:s', 'activate:list', 'activate:collection', 'activate:l', 'activate:c',
                'active:set', 'active:s', 'active:list', 'active:collection', 'active:l', 'active:c',
                'a:set', 'a:s', 'a:list', 'a:collection', 'a:l', 'a:c',
                'deactivate:set', 'deactivate:s', 'deactivate:list', 'deactivate:collection', 'deactivate:l', 'deactivate:c',
                'deactive:set', 'deactive:s', 'deactive:list', 'deactive:collection', 'deactive:l', 'deactive:c',
                'd:set', 'd:s', 'd:list', 'd:collection', 'd:l', 'd:c'
            ) } { $kind = 'sets'; break }
            { $_ -in @(
                'activate:up', 'activate:upper', 'activate:u',
                'active:up', 'active:upper', 'active:u',
                'a:up', 'a:upper', 'a:u',
                'deactivate:up', 'deactivate:upper', 'deactivate:u',
                'deactive:up', 'deactive:upper', 'deactive:u',
                'd:up', 'd:upper', 'd:u'
            ) } { $kind = 'ups'; break }
            { $_ -in @(
                'activate:account', 'activate:a', 'active:account', 'active:a', 'a:account', 'a:a',
                'deactivate:account', 'deactivate:a', 'deactive:account', 'deactive:a', 'd:account', 'd:a',
                'auth:logout', 'auth:check'
            ) } { $kind = 'accounts'; break }
            { $_ -like 'pull:*' -or $_ -like 'p:*' -or $_ -like 'like:*' -or $_ -eq 'media:reset' -or $_ -eq 'media:skip' } {
                $kind = 'medias'; break
            }
        }
    }

    if ($kind) {
        $results = @()
        foreach ($line in (& '__PS_BIN__' @dirArgs __complete $kind $wordToComplete 2>$null)) {
            if (-not $line) { continue }
            $parts = $line -split "`t", 2
            $id = $parts[0]
            if (-not $id) { continue }
            $desc = if ($parts.Length -gt 1 -and $parts[1]) { $parts[1] } else { $id }
            $listText = if ($desc -ne $id) { "$id ($desc)" } else { $id }
            $results += [System.Management.Automation.CompletionResult]::new($id, $listText, 'ParameterValue', $desc)
        }
        return $results
    }

    return (__FALLBACK_CALL__)
}.GetNewClosure())
"#
    .replace("__RAW_BIN__", bin_name)
    .replace("__PS_BIN__", &ps_bin)
    .replace("__FALLBACK_CALL__", &fallback_call);

    let mut out = base_script;
    out.push_str(&dynamic_block);
    out
}

fn attach_nushell_completer(line: &str, completer: &str) -> String {
    if line.contains("@\"") {
        return line.to_owned();
    }
    let (code_part, comment_part) = match line.find('#') {
        Some(idx) => (&line[..idx], &line[idx..]),
        None => (line, ""),
    };
    let trimmed_end = code_part.trim_end();
    let trailing_ws = &code_part[trimmed_end.len()..];
    let updated_code = if let Some(prefix) = trimmed_end.strip_suffix(": string") {
        format!("{prefix}: string@\"{completer}\"")
    } else if let Some(prefix) = trimmed_end.strip_suffix(": int") {
        format!("{prefix}: string@\"{completer}\"")
    } else if !trimmed_end.contains(':') {
        format!("{trimmed_end}: string@\"{completer}\"")
    } else {
        trimmed_end.to_owned()
    };
    format!("{updated_code}{trailing_ws}{comment_part}")
}

fn wire_nushell_dynamic_completers(script: &str, bin_name: &str) -> String {
    let defs = format!(
        "  # Dynamic workspace DB completers via `{bin_name} __complete`\n\
         \x20 def \"nu-complete {bin_name} sets\" [] {{\n\
         \x20   ^{bin_name} __complete sets \"\" | lines | parse \"{{value}}\\t{{description}}\"\n\
         \x20 }}\n\
         \n\
         \x20 def \"nu-complete {bin_name} ups\" [] {{\n\
         \x20   ^{bin_name} __complete ups \"\" | lines | parse \"{{value}}\\t{{description}}\"\n\
         \x20 }}\n\
         \n\
         \x20 def \"nu-complete {bin_name} accounts\" [] {{\n\
         \x20   ^{bin_name} __complete accounts \"\" | lines | parse \"{{value}}\\t{{description}}\"\n\
         \x20 }}\n\
         \n\
         \x20 def \"nu-complete {bin_name} medias\" [] {{\n\
         \x20   ^{bin_name} __complete medias \"\" | lines | parse \"{{value}}\\t{{description}}\"\n\
         \x20 }}\n"
    );

    let mut out_lines = Vec::new();
    let mut inserted_defs = false;
    let mut current_subcmd: Option<String> = None;

    for line in script.lines() {
        let trimmed = line.trim();

        if !inserted_defs && trimmed == "module completions {" {
            out_lines.push(line.to_owned());
            out_lines.push(String::new());
            out_lines.push(defs.clone());
            inserted_defs = true;
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("export extern ")
            && let Some(cmd_part) = rest.strip_suffix('[')
        {
            let cmd_name = cmd_part.trim().trim_matches('"');
            let subcmd = cmd_name
                .strip_prefix(bin_name)
                .map(str::trim)
                .unwrap_or("")
                .to_owned();
            current_subcmd = Some(subcmd);
            out_lines.push(line.to_owned());
            continue;
        }

        if trimmed == "]" {
            current_subcmd = None;
            out_lines.push(line.to_owned());
            continue;
        }

        if let Some(subcmd) = current_subcmd.as_deref() {
            let mut completer_kind: Option<&str> = None;
            if trimmed.starts_with("--set:") || trimmed.starts_with("--set ") {
                completer_kind = Some("sets");
            } else if trimmed.starts_with("--up:") || trimmed.starts_with("--up ") {
                completer_kind = Some("ups");
            } else if trimmed.starts_with("--account:") || trimmed.starts_with("--account ") {
                completer_kind = Some("accounts");
            } else if trimmed.starts_with("...targets")
                || trimmed.starts_with("targets")
                || trimmed.starts_with("...account_id")
                || trimmed.starts_with("account_id")
            {
                completer_kind = match subcmd {
                    "activate set" | "deactivate set" => Some("sets"),
                    "activate up" | "deactivate up" => Some("ups"),
                    "activate account" | "deactivate account" | "auth logout" | "auth check" => {
                        Some("accounts")
                    }
                    "pull" | "like" | "media reset" | "media skip" => Some("medias"),
                    _ => None,
                };
            }

            if let Some(kind) = completer_kind {
                let completer = format!("nu-complete {bin_name} {kind}");
                out_lines.push(attach_nushell_completer(line, &completer));
                continue;
            }
        }

        out_lines.push(line.to_owned());
    }

    let mut out = out_lines.join("\n");
    out.push('\n');
    if !inserted_defs {
        out.push('\n');
        out.push_str(&defs);
    }
    out
}

pub fn generate_completion_script(
    cmd: &mut Command,
    shell: CompletionShell,
    bin_name: &str,
    static_only: bool,
) -> String {
    let mut buf = Vec::new();
    match shell {
        CompletionShell::Bash => clap_complete::generate(Shell::Bash, cmd, bin_name, &mut buf),
        CompletionShell::Zsh => clap_complete::generate(Shell::Zsh, cmd, bin_name, &mut buf),
        CompletionShell::Fish => clap_complete::generate(Shell::Fish, cmd, bin_name, &mut buf),
        CompletionShell::PowerShell => {
            clap_complete::generate(Shell::PowerShell, cmd, bin_name, &mut buf)
        }
        CompletionShell::Elvish => clap_complete::generate(Shell::Elvish, cmd, bin_name, &mut buf),
        CompletionShell::Nushell => {
            clap_complete::generate(clap_complete_nushell::Nushell, cmd, bin_name, &mut buf)
        }
    }
    let mut script = String::from_utf8(buf).unwrap_or_default();
    if !static_only {
        match shell {
            CompletionShell::Zsh => {
                let fn_ident = sanitize_shell_fn_ident(bin_name);
                let clap_tail = format!(
                    "\nif [ \"$funcstack[1]\" = \"_{fn_ident}\" ]; then\n    _{fn_ident} \"$@\"\nelse\n    compdef _{fn_ident} {bin_name}\nfi\n"
                );
                if let Some(stripped) = script.strip_suffix(&clap_tail) {
                    script = stripped.to_owned();
                }
                script.push_str(&dynamic_completion_snippet(shell, bin_name));
            }
            CompletionShell::PowerShell => {
                script = wire_powershell_dynamic_completer(&script, bin_name);
            }
            CompletionShell::Nushell => {
                script = wire_nushell_dynamic_completers(&script, bin_name);
            }
            _ => {
                let snippet = dynamic_completion_snippet(shell, bin_name);
                if !snippet.is_empty() {
                    script.push_str(&snippet);
                }
            }
        }
    }
    script
}

fn sanitize_completion_desc(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\t' | '\r' | '\n' => out.push(' '),
            c if c.is_control() => {}
            other => out.push(other),
        }
    }
    out
}

/// Query dynamic completion lines (`id\tdescription`) from `db` for `kind` filtered by `prefix`.
pub async fn dynamic_complete_lines(db: &Db, kind: &str, prefix: &str) -> Vec<String> {
    let needle = prefix.trim().to_ascii_lowercase();
    let matches_prefix = |id: &str, label: &str| {
        needle.is_empty()
            || id.to_ascii_lowercase().starts_with(&needle)
            || label.to_ascii_lowercase().contains(&needle)
    };

    let mut out = Vec::new();
    match kind {
        "sets" => {
            if let Ok(rows) = db.all_sets().await {
                for s in rows {
                    let id = s.set_id.to_string();
                    if matches_prefix(&id, &s.name) {
                        out.push(format!(
                            "{id}\t{} ({}, {})",
                            sanitize_completion_desc(&s.name),
                            s.kind,
                            s.state
                        ));
                    }
                }
            }
        }
        "ups" => {
            if let Ok(rows) = db.all_ups().await {
                for u in rows {
                    let id = u.up_id.to_string();
                    if matches_prefix(&id, &u.name) {
                        out.push(format!(
                            "{id}\t{} ({})",
                            sanitize_completion_desc(&u.name),
                            u.state
                        ));
                    }
                }
            }
        }
        "accounts" => {
            if let Ok(rows) = db.all_accounts().await {
                for a in rows {
                    let id = a.account_id.to_string();
                    if matches_prefix(&id, &a.name) {
                        out.push(format!(
                            "{id}\t{} ({})",
                            sanitize_completion_desc(&a.name),
                            a.state
                        ));
                    }
                }
            }
        }
        "medias" => {
            if let Ok(rows) = db.all_medias().await {
                for m in rows
                    .into_iter()
                    .filter(|m| {
                        matches_prefix(&m.bv_id, &m.title)
                            || matches_prefix(&m.id.to_string(), &m.title)
                    })
                    .take(200)
                {
                    out.push(format!(
                        "{}\t{}",
                        m.bv_id,
                        sanitize_completion_desc(&m.title)
                    ));
                }
            }
        }
        _ => {}
    }
    out
}

pub async fn run_dynamic_complete(
    runtime: RuntimeOptions,
    kind: &str,
    prefix: &str,
) -> AppResult<()> {
    let Ok(ctx) = AppContext::open_read_only(runtime).await else {
        return Ok(());
    };
    for line in dynamic_complete_lines(&ctx.db, kind, prefix).await {
        if !write_stdout_line(&line).unwrap_or(false) {
            break;
        }
    }
    Ok(())
}

/// Parse CLI arguments from `std::env::args_os()` and execute the selected subcommand.
pub async fn run() -> AppResult<()> {
    let mut cmd = build_command();
    let matches = cmd.get_matches_mut();
    dispatch(&mut cmd, matches).await
}

/// Dispatch an already-parsed command invocation.
async fn dispatch(cmd: &mut Command, matches: ArgMatches) -> AppResult<()> {
    let runtime = runtime_from_matches(&matches);

    match matches.subcommand() {
        Some(("completion", sub)) => {
            let shell = *sub
                .get_one::<CompletionShell>("shell")
                .ok_or_else(|| AppError::usage(anyhow!("missing required <shell> argument")))?;
            let bin_name = sub
                .get_one::<String>("bin-name")
                .cloned()
                .unwrap_or_else(|| "fav".to_owned());
            let static_only = sub.get_flag("static");

            let script = generate_completion_script(cmd, shell, &bin_name, static_only);
            write_stdout_all(&script).map_err(AppError::runtime)?;
            return Ok(());
        }
        Some(("__complete", sub)) => {
            let kind = sub
                .get_one::<String>("kind")
                .map(String::as_str)
                .unwrap_or_default();
            let prefix = sub
                .get_one::<String>("prefix")
                .map(String::as_str)
                .unwrap_or_default();
            return run_dynamic_complete(runtime, kind, prefix).await;
        }
        _ => {}
    }

    init_tracing(&runtime);

    match matches.subcommand() {
        Some(("init", _)) => {
            let ctx = AppContext::open_read_write(runtime, true).await?;
            info!(
                "Initialized fav workspace at '{}' (db: '{}').",
                ctx.workspace.root().display(),
                ctx.workspace.db_path().display()
            );
            ctx.hint("Run `fav auth login` or `fav auth usecookies --stdin` to add an account.");
            Ok(())
        }
        Some(("schedule", sub)) => {
            let target = *sub
                .get_one::<ScheduleTarget>("target")
                .ok_or_else(|| AppError::usage(anyhow!("missing schedule target")))?;
            let interval = sub
                .get_one::<String>("interval")
                .map(String::as_str)
                .unwrap_or("3h");
            let name = sub.get_one::<String>("name").map(String::as_str);
            let install = sub.get_flag("install");
            let ws = Workspace::discover_or_init(runtime.explicit_dir.as_deref())?;
            let ws_root =
                std::fs::canonicalize(ws.root()).unwrap_or_else(|_| ws.root().to_path_buf());
            let fav_bin = sub
                .get_one::<PathBuf>("bin")
                .cloned()
                .or_else(|| std::env::current_exe().ok())
                .unwrap_or_else(|| PathBuf::from("fav"));
            let output = schedule_named(target, interval, &ws_root, &fav_bin, install, name)?;
            write_stdout_line(&output).map_err(AppError::runtime)?;
            Ok(())
        }
        Some(("config", sub)) => match sub.subcommand() {
            None => config_show(&runtime, resolve_format(sub)),
            Some(("show", s)) => config_show(&runtime, resolve_subcommand_format(sub, s)?),
            Some(("path", s)) => config_path(&runtime, resolve_subcommand_format(sub, s)?),
            _ => Err(AppError::usage(anyhow!("unknown config subcommand"))),
        },
        Some(("auth", sub)) => match sub.subcommand() {
            Some(("login", s)) => {
                let ctx = AppContext::open_read_write(runtime, true).await?;
                let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
                login(
                    &ctx,
                    LoginOptions {
                        invert_qr: s.get_flag("invert"),
                        show_url: s.get_flag("url"),
                        fetch: s.get_flag("fetch"),
                    },
                )
                .await
            }
            Some(("usecookies", s)) => {
                let ctx = AppContext::open_read_write(runtime, true).await?;
                let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
                use_cookies(
                    &ctx,
                    UseCookiesOptions {
                        cookies: collect_strings(s, "cookies"),
                        stdin: s.get_flag("stdin"),
                        file: s.get_one::<PathBuf>("file").cloned(),
                        fetch: s.get_flag("fetch"),
                    },
                )
                .await
            }
            Some(("logout", s)) => {
                let ctx = AppContext::open_read_write(runtime, false).await?;
                let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
                logout(
                    &ctx,
                    LogoutOptions {
                        account_ids: collect_i64s(s, "account_id"),
                        all: s.get_flag("all"),
                        local_only: s.get_flag("local"),
                        force: s.get_flag("force"),
                        yes: s.get_flag("yes"),
                    },
                )
                .await
            }
            Some(("check", s)) => {
                // `auth check` may persist refreshed cookies or mark expired accounts
                let ctx = AppContext::open_read_write(runtime, false).await?;
                let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
                check(
                    &ctx,
                    CheckOptions {
                        account_ids: collect_i64s(s, "account_id"),
                        all: s.get_flag("all"),
                        format: resolve_format(s),
                        wide: s.get_flag("wide"),
                        refresh: s.get_flag("refresh"),
                    },
                )
                .await
            }
            _ => Err(AppError::usage(anyhow!("missing auth subcommand"))),
        },
        Some(("list", sub)) => {
            let ctx = AppContext::open_read_only(runtime).await?;
            match sub.subcommand() {
                None => status(&ctx, resolve_format(sub), 10).await,
                Some(("account", s)) => list(&ctx, ListKind::Account, parse_list_options(s)).await,
                Some(("set", s)) => list(&ctx, ListKind::Set, parse_list_options(s)).await,
                Some(("up", s)) => list(&ctx, ListKind::Up, parse_list_options(s)).await,
                Some(("media", s)) => list(&ctx, ListKind::Media, parse_list_options(s)).await,
                _ => Err(AppError::usage(anyhow!("unknown list target"))),
            }
        }
        Some(("activate", sub)) => {
            let ctx = AppContext::open_read_write(runtime, false).await?;
            let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
            match sub.subcommand() {
                Some(("account", s)) => {
                    activate(
                        &ctx,
                        TargetKind::Account,
                        ActivateOptions {
                            targets: collect_strings(s, "targets"),
                            all: s.get_flag("all"),
                            fetch: s.get_flag("fetch"),
                        },
                    )
                    .await
                }
                Some(("set", s)) => {
                    activate(
                        &ctx,
                        TargetKind::Set,
                        ActivateOptions {
                            targets: collect_strings(s, "targets"),
                            all: s.get_flag("all"),
                            fetch: s.get_flag("fetch"),
                        },
                    )
                    .await
                }
                Some(("up", s)) => {
                    activate(
                        &ctx,
                        TargetKind::Up,
                        ActivateOptions {
                            targets: collect_strings(s, "targets"),
                            all: s.get_flag("all"),
                            fetch: s.get_flag("fetch"),
                        },
                    )
                    .await
                }
                Some(("all", s)) => {
                    activate(
                        &ctx,
                        TargetKind::All,
                        ActivateOptions {
                            targets: Vec::new(),
                            all: true,
                            fetch: s.get_flag("fetch"),
                        },
                    )
                    .await
                }
                _ => Err(AppError::usage(anyhow!("missing activate target"))),
            }
        }
        Some(("deactivate", sub)) => {
            let ctx = AppContext::open_read_write(runtime, false).await?;
            match sub.subcommand() {
                Some(("account", s)) => {
                    deactivate(
                        &ctx,
                        TargetKind::Account,
                        DeactivateOptions {
                            targets: collect_strings(s, "targets"),
                            all: s.get_flag("all"),
                        },
                    )
                    .await
                }
                Some(("set", s)) => {
                    deactivate(
                        &ctx,
                        TargetKind::Set,
                        DeactivateOptions {
                            targets: collect_strings(s, "targets"),
                            all: s.get_flag("all"),
                        },
                    )
                    .await
                }
                Some(("up", s)) => {
                    deactivate(
                        &ctx,
                        TargetKind::Up,
                        DeactivateOptions {
                            targets: collect_strings(s, "targets"),
                            all: s.get_flag("all"),
                        },
                    )
                    .await
                }
                Some(("all", _)) => {
                    deactivate(
                        &ctx,
                        TargetKind::All,
                        DeactivateOptions {
                            targets: Vec::new(),
                            all: true,
                        },
                    )
                    .await
                }
                _ => Err(AppError::usage(anyhow!("missing deactivate target"))),
            }
        }
        Some(("fetch", sub)) => {
            let ctx = AppContext::open_read_write(runtime, false).await?;
            let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
            fetch(&ctx, parse_fetch_options(sub)).await
        }
        Some(("pull", sub)) => {
            let opts = parse_pull_options(sub, true)?;
            let ctx = AppContext::open_read_write(runtime, false).await?;
            let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
            pull(&ctx, opts).await
        }
        Some(("sync", sub)) => {
            let fetch_opts = parse_fetch_options(sub);
            let pull_opts = parse_pull_options(sub, false)?;
            let ctx = AppContext::open_read_write(runtime, false).await?;
            let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
            sync(
                &ctx,
                SyncOptions {
                    fetch: fetch_opts,
                    pull: pull_opts,
                },
            )
            .await
        }
        Some(("download", sub)) => {
            let opts = parse_download_options(sub)?;
            download(runtime, opts).await
        }
        Some(("like", sub)) => {
            let raw_targets = collect_strings(sub, "targets");
            let mut read_stdin = sub.get_flag("stdin");
            let mut targets = Vec::with_capacity(raw_targets.len());
            for t in raw_targets {
                if t == "-" {
                    read_stdin = true;
                } else {
                    targets.push(t);
                }
            }
            let ctx = AppContext::open_read_write(runtime, false).await?;
            let _guard = ProgressSlotGuard::install(ctx.multi_progress.clone());
            like(
                &ctx,
                LikeOptions {
                    targets,
                    set_ids: collect_i64s(sub, "set"),
                    all_active: sub.get_flag("all-active"),
                    unlike: sub.get_flag("unlike"),
                    account_ids: collect_i64s(sub, "account"),
                    dry_run: sub.get_flag("dry-run"),
                    read_stdin,
                    ignore_errors: sub.get_flag("ignore-errors"),
                },
            )
            .await
        }
        Some(("status", sub)) => {
            let ctx = AppContext::open_read_only(runtime).await?;
            let top = sub.get_one::<usize>("top").copied().unwrap_or(10);
            status(&ctx, resolve_format(sub), top).await
        }
        Some(("doctor", sub)) => {
            let ctx = AppContext::open_read_only(runtime).await?;
            doctor_with_options(
                &ctx,
                DoctorOptions {
                    format: resolve_format(sub),
                    offline: sub.get_flag("offline"),
                    output_dir: sub.get_one::<PathBuf>("output").cloned(),
                },
            )
            .await
        }
        Some(("media", sub)) => {
            let ctx = AppContext::open_read_write(runtime, false).await?;
            match sub.subcommand() {
                Some(("reset", s)) => {
                    media_reset(
                        &ctx,
                        MediaResetOptions {
                            targets: collect_strings(s, "targets"),
                            failed: s.get_flag("failed"),
                            expired: s.get_flag("expired"),
                            permission_denied: s.get_flag("permission-denied"),
                            skipped: s.get_flag("skipped"),
                            completed: s.get_flag("completed"),
                            missing: s.get_flag("missing"),
                            include_inactive: s.get_flag("include-inactive"),
                            output_dir: s.get_one::<PathBuf>("output").cloned(),
                            all: s.get_flag("all"),
                            dry_run: s.get_flag("dry-run"),
                            yes: s.get_flag("yes"),
                        },
                    )
                    .await
                }
                Some(("skip", s)) => media_skip(&ctx, collect_strings(s, "targets")).await,
                _ => Err(AppError::usage(anyhow!("missing media subcommand"))),
            }
        }
        _ => Err(AppError::usage(anyhow!("missing subcommand"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_command_tree_is_valid() {
        build_command().debug_assert();
    }

    #[test]
    fn global_flags_work_after_subcommands() {
        let cmd = build_command();
        let m = cmd
            .try_get_matches_from(["fav", "pull", "-v", "-q", "-C", "/tmp/ws", "--no-progress"])
            .unwrap();
        let rt = runtime_from_matches(&m);
        assert_eq!(rt.verbose, 1);
        assert!(rt.quiet);
        assert_eq!(rt.explicit_dir, Some(PathBuf::from("/tmp/ws")));
        assert!(rt.no_progress);
    }

    #[test]
    fn help_renders_fav_binary_name_and_fixed_typos() {
        let mut cmd = build_command();
        let mut buf = Vec::new();
        cmd.write_long_help(&mut buf).unwrap();
        let help = String::from_utf8(buf).unwrap();
        assert!(help.contains("Usage: fav"));
        assert!(!help.contains("fav_bili"));

        let mut deact = build_command();
        let acct = deact
            .find_subcommand_mut("deactivate")
            .unwrap()
            .find_subcommand_mut("account")
            .unwrap();
        let mut buf = Vec::new();
        acct.write_long_help(&mut buf).unwrap();
        let deact_help = String::from_utf8(buf).unwrap();
        assert!(!deact_help.contains("Dectivate"));
    }

    #[test]
    fn aliases_and_bare_ls_parse_cleanly() {
        // Bare `fav ls` parses cleanly (invokes status)
        let m = build_command()
            .try_get_matches_from(["fav", "ls", "--json"])
            .unwrap();
        assert_eq!(m.subcommand_name(), Some("list"));

        // `upper` alias works on activate/deactivate up
        let m = build_command()
            .try_get_matches_from(["fav", "a", "upper", "123", "--fetch"])
            .unwrap();
        assert_eq!(m.subcommand_name(), Some("activate"));

        let m = build_command()
            .try_get_matches_from(["fav", "d", "upper", "-a"])
            .unwrap();
        assert_eq!(m.subcommand_name(), Some("deactivate"));

        // `s` alias for sync
        let m = build_command()
            .try_get_matches_from(["fav", "s", "-p", "-j", "2", "--limit-rate", "5M"])
            .unwrap();
        assert_eq!(m.subcommand_name(), Some("sync"));
    }

    #[test]
    fn new_download_pull_sync_fetch_list_doctor_media_flags_parse_cleanly() {
        // `fav download` / `fav dl` full flag coverage + numeric qn + `-` stdin
        let m = build_command()
            .try_get_matches_from([
                "fav",
                "dl",
                "BV1Q541167Qg",
                "-",
                "--page",
                "1,3-5",
                "--audio-only",
                "--audio-format",
                "mp3",
                "--quality",
                "120",
                "--codec",
                "h265",
                "-o",
                "./out",
                "-j",
                "2",
                "--limit-rate",
                "2M",
                "--force",
                "--anonymous",
                "--dry-run",
                "--ignore-errors",
            ])
            .unwrap();
        assert_eq!(m.subcommand_name(), Some("download"));
        let opts = parse_download_options(m.subcommand_matches("download").unwrap()).unwrap();
        assert_eq!(opts.targets, vec!["BV1Q541167Qg"]);
        assert!(opts.read_stdin);
        assert_eq!(opts.pages.as_deref(), Some("1,3-5"));
        assert!(!opts.all_pages);
        assert!(opts.sound_only);
        assert_eq!(opts.audio_format, Some(AudioFormat::Mp3));
        assert_eq!(opts.clarity, Some(VideoClarity::FourK));
        assert_eq!(opts.codec, Some(VideoCodec::Hevc));
        assert_eq!(opts.output_dir, Some(PathBuf::from("./out")));
        assert_eq!(opts.jobs, Some(2));
        assert_eq!(opts.limit_rate, Some(2 * 1024 * 1024));
        assert!(opts.force);
        assert!(opts.anonymous);
        assert!(opts.dry_run);
        assert!(opts.ignore_errors);

        // `--page` conflicts with `--all-pages`
        assert!(
            build_command()
                .try_get_matches_from([
                    "fav",
                    "download",
                    "BV1Q541167Qg",
                    "--page",
                    "1",
                    "--all-pages"
                ])
                .is_err()
        );

        // `--audio-format` parses without `--sound-only` so `sound_only = true` in config.toml
        // works; runtime validation enforces that effective sound_only is enabled.
        let m = build_command()
            .try_get_matches_from(["fav", "download", "BV1Q541167Qg", "--audio-format", "flac"])
            .unwrap();
        let dl_opts = parse_download_options(m.subcommand_matches("download").unwrap()).unwrap();
        assert!(!dl_opts.sound_only);
        assert_eq!(dl_opts.audio_format, Some(AudioFormat::Flac));

        let m = build_command()
            .try_get_matches_from(["fav", "pull", "--audio-format", "opus"])
            .unwrap();
        let p_opts = parse_pull_options(m.subcommand_matches("pull").unwrap(), true).unwrap();
        assert!(!p_opts.sound_only);
        assert_eq!(p_opts.audio_format, Some(AudioFormat::Opus));

        let m = build_command()
            .try_get_matches_from(["fav", "sync", "--audio-format", "mp3"])
            .unwrap();
        let s_opts = parse_pull_options(m.subcommand_matches("sync").unwrap(), false).unwrap();
        assert!(!s_opts.sound_only);
        assert_eq!(s_opts.audio_format, Some(AudioFormat::Mp3));

        // `--all-pages` + canonical clarity name
        let m = build_command()
            .try_get_matches_from([
                "fav",
                "download",
                "https://www.bilibili.com/video/BV1Q541167Qg/?p=2",
                "--all-pages",
                "--clarity",
                "1080p60",
            ])
            .unwrap();
        let opts = parse_download_options(m.subcommand_matches("download").unwrap()).unwrap();
        assert!(opts.all_pages);
        assert_eq!(opts.clarity, Some(VideoClarity::P1080HighFrame));

        // `pull` and `sync` accept new stream + recheck flags
        let m = build_command()
            .try_get_matches_from([
                "fav",
                "pull",
                "--sound-only",
                "--audio-format",
                "opus",
                "--clarity",
                "80",
                "--codec",
                "av1",
                "--recheck",
            ])
            .unwrap();
        let pull_opts = parse_pull_options(m.subcommand_matches("pull").unwrap(), true).unwrap();
        assert!(pull_opts.sound_only);
        assert_eq!(pull_opts.audio_format, Some(AudioFormat::Opus));
        assert_eq!(pull_opts.clarity, Some(VideoClarity::P1080));
        assert_eq!(pull_opts.codec, Some(VideoCodec::Av1));
        assert!(pull_opts.recheck);

        let m = build_command()
            .try_get_matches_from([
                "fav",
                "sync",
                "--refresh-status",
                "--clarity",
                "720p",
                "--codec",
                "h264",
                "--recheck",
            ])
            .unwrap();
        let sync_sub = m.subcommand_matches("sync").unwrap();
        assert!(parse_fetch_options(sync_sub).full);
        let sync_pull = parse_pull_options(sync_sub, false).unwrap();
        assert_eq!(sync_pull.clarity, Some(VideoClarity::P720));
        assert_eq!(sync_pull.codec, Some(VideoCodec::Avc));
        assert!(sync_pull.recheck);

        // `ls set --kind`
        let m = build_command()
            .try_get_matches_from(["fav", "ls", "set", "--kind", "season"])
            .unwrap();
        let list_opts = parse_list_options(
            m.subcommand_matches("list")
                .unwrap()
                .subcommand_matches("set")
                .unwrap(),
        );
        assert_eq!(list_opts.kind.as_deref(), Some("season"));
        assert!(!list_opts.followed_only);
        assert!(
            build_command()
                .try_get_matches_from(["fav", "ls", "set", "--kind", "invalid"])
                .is_err()
        );

        // `ls up --followed`
        let m = build_command()
            .try_get_matches_from(["fav", "ls", "up", "--followed"])
            .unwrap();
        let up_opts = parse_list_options(
            m.subcommand_matches("list")
                .unwrap()
                .subcommand_matches("up")
                .unwrap(),
        );
        assert!(up_opts.followed_only);
        assert!(up_opts.has_filters());

        let m = build_command()
            .try_get_matches_from(["fav", "ls", "up"])
            .unwrap();
        let up_opts_default = parse_list_options(
            m.subcommand_matches("list")
                .unwrap()
                .subcommand_matches("up")
                .unwrap(),
        );
        assert!(!up_opts_default.followed_only);

        // `doctor -o` and `media reset -o` / `-y` / `--yes`
        let m = build_command()
            .try_get_matches_from(["fav", "doctor", "--offline", "-o", "/tmp/videos"])
            .unwrap();
        assert_eq!(
            m.subcommand_matches("doctor")
                .unwrap()
                .get_one::<PathBuf>("output"),
            Some(&PathBuf::from("/tmp/videos"))
        );

        let m = build_command()
            .try_get_matches_from([
                "fav",
                "media",
                "reset",
                "--missing",
                "--include-inactive",
                "-o",
                "/tmp/videos",
                "-y",
            ])
            .unwrap();
        let reset_m = m
            .subcommand_matches("media")
            .unwrap()
            .subcommand_matches("reset")
            .unwrap();
        assert_eq!(
            reset_m.get_one::<PathBuf>("output"),
            Some(&PathBuf::from("/tmp/videos"))
        );
        assert!(reset_m.get_flag("include-inactive"));
        assert!(reset_m.get_flag("yes"));

        let m = build_command()
            .try_get_matches_from(["fav", "media", "reset", "--completed", "--yes"])
            .unwrap();
        let reset_yes = m
            .subcommand_matches("media")
            .unwrap()
            .subcommand_matches("reset")
            .unwrap();
        assert!(reset_yes.get_flag("completed"));
        assert!(reset_yes.get_flag("yes"));

        // `completion nushell` and alias `nu` and `powershell`
        for (arg, expected) in [
            ("nushell", CompletionShell::Nushell),
            ("nu", CompletionShell::Nushell),
            ("powershell", CompletionShell::PowerShell),
            ("pwsh", CompletionShell::PowerShell),
        ] {
            let m = build_command()
                .try_get_matches_from(["fav", "completion", arg])
                .unwrap();
            let shell = *m
                .subcommand_matches("completion")
                .unwrap()
                .get_one::<CompletionShell>("shell")
                .unwrap();
            assert_eq!(shell, expected);
        }

        // `schedule --name`
        let m = build_command()
            .try_get_matches_from(["fav", "schedule", "--target", "systemd", "--name", "work"])
            .unwrap();
        let sched_m = m.subcommand_matches("schedule").unwrap();
        assert_eq!(
            sched_m.get_one::<String>("name").map(String::as_str),
            Some("work")
        );

        // `config` (bare), `config show`, and `config path` with format flags
        let m = build_command()
            .try_get_matches_from(["fav", "config", "--json"])
            .unwrap();
        let cfg_m = m.subcommand_matches("config").unwrap();
        assert!(cfg_m.subcommand().is_none());
        assert_eq!(resolve_format(cfg_m), OutputFormat::Json);

        let m = build_command()
            .try_get_matches_from(["fav", "config", "show", "--format", "tsv"])
            .unwrap();
        let show_m = m
            .subcommand_matches("config")
            .unwrap()
            .subcommand_matches("show")
            .unwrap();
        assert_eq!(resolve_format(show_m), OutputFormat::Tsv);

        let m = build_command()
            .try_get_matches_from(["fav", "config", "path", "--json"])
            .unwrap();
        let path_m = m
            .subcommand_matches("config")
            .unwrap()
            .subcommand_matches("path")
            .unwrap();
        assert_eq!(resolve_format(path_m), OutputFormat::Json);
    }

    #[test]
    fn completion_scripts_wire_dynamic_wrappers_and_respect_static_flag() {
        let mut cmd = build_command();

        // Bash dynamic & static + -C forwarding + new value-taking flags
        let bash_dyn = generate_completion_script(&mut cmd, CompletionShell::Bash, "fav", false);
        assert!(bash_dyn.contains("_fav_with_dynamic()"));
        assert!(bash_dyn.contains("complete -F _fav_with_dynamic"));
        assert!(bash_dyn.contains("dir_args=(-C"));
        assert!(bash_dyn.contains("--clarity|--quality|--codec|--audio-format|--page"));
        assert!(bash_dyn.contains("--kind"));
        assert!(bash_dyn.contains("__complete"));
        assert!(bash_dyn.contains("1080p60"));

        let bash_static = generate_completion_script(&mut cmd, CompletionShell::Bash, "fav", true);
        assert!(!bash_static.contains("_fav_with_dynamic"));
        assert!(bash_static.contains("1080p60"));

        // Zsh dynamic & static + -C forwarding
        let zsh_dyn = generate_completion_script(&mut cmd, CompletionShell::Zsh, "fav", false);
        assert!(zsh_dyn.contains("_fav_with_dynamic()"));
        assert!(zsh_dyn.contains("compdef _fav_with_dynamic fav"));
        assert!(!zsh_dyn.contains("compdef _fav fav"));
        assert!(zsh_dyn.contains("dir_args=(-C"));
        assert!(zsh_dyn.contains("--clarity|--quality|--codec|--audio-format|--page"));

        let zsh_static = generate_completion_script(&mut cmd, CompletionShell::Zsh, "fav", true);
        assert!(!zsh_static.contains("_fav_with_dynamic"));
        assert!(zsh_static.contains("compdef _fav fav"));

        // Fish dynamic & static + -C forwarding + positional subcommand matching (no `a` alias collision)
        let fish_dyn = generate_completion_script(&mut cmd, CompletionShell::Fish, "fav", false);
        assert!(fish_dyn.contains("function __fish_fav_complete"));
        assert!(fish_dyn.contains("function __fish_fav_subcmd_is"));
        assert!(fish_dyn.contains("set dir_args -C"));
        assert!(fish_dyn.contains("fav $dir_args __complete $kind"));
        assert!(fish_dyn.contains("__fish_fav_complete sets"));
        assert!(fish_dyn.contains("__fish_fav_subcmd_is sets"));
        assert!(fish_dyn.contains("__fish_fav_subcmd_is ups"));
        assert!(fish_dyn.contains("__fish_fav_subcmd_is accounts"));
        assert!(fish_dyn.contains("__fish_fav_subcmd_is medias"));
        assert!(fish_dyn.contains("--clarity --quality --codec --audio-format --page"));
        assert!(fish_dyn.contains("-l set"));
        // Dynamic snippet must NOT use unordered `__fish_seen_subcommand_from` (which caused `a` collision)
        let fish_dynamic_section = fish_dyn
            .split_once("# Dynamic workspace DB completions")
            .map(|(_, rest)| rest)
            .expect("missing Fish dynamic completion section");
        assert!(
            !fish_dynamic_section.contains("__fish_seen_subcommand_from"),
            "Fish dynamic completions must use positional sub1/sub2 checks instead of __fish_seen_subcommand_from"
        );
        assert!(fish_dynamic_section.contains("set -l sub1"));
        assert!(fish_dynamic_section.contains("set -l sub2"));

        let fish_static = generate_completion_script(&mut cmd, CompletionShell::Fish, "fav", true);
        assert!(!fish_static.contains("__fish_fav_complete"));
        assert!(!fish_static.contains("__fish_fav_subcmd_is"));

        // PowerShell genuine dynamic Register-ArgumentCompleter + -C forwarding + static fallback
        let ps_dyn =
            generate_completion_script(&mut cmd, CompletionShell::PowerShell, "fav", false);
        assert!(ps_dyn.contains("$__fav_static_fav = {"));
        assert!(ps_dyn.contains("Register-ArgumentCompleter -Native -CommandName 'fav'"));
        assert!(ps_dyn.contains("$dirArgs = @('-C'"));
        assert!(ps_dyn.contains("& 'fav' @dirArgs __complete $kind $wordToComplete"));
        assert!(ps_dyn.contains("[System.Management.Automation.CompletionResult]::new("));
        assert!(ps_dyn.contains("& $__fav_static_fav $wordToComplete $commandAst $cursorPosition"));
        assert!(ps_dyn.contains("'--clarity'"));
        assert!(ps_dyn.contains("'--audio-format'"));

        let ps_static =
            generate_completion_script(&mut cmd, CompletionShell::PowerShell, "fav", true);
        assert!(!ps_static.contains("$__fav_static_fav"));
        assert!(!ps_static.contains("@dirArgs __complete"));
        assert!(ps_static.contains("Register-ArgumentCompleter"));

        // Nushell dynamic & static
        let nu_dyn = generate_completion_script(&mut cmd, CompletionShell::Nushell, "fav", false);
        assert!(nu_dyn.contains("def \"nu-complete fav sets\" []"));
        assert!(nu_dyn.contains("def \"nu-complete fav ups\" []"));
        assert!(nu_dyn.contains("def \"nu-complete fav accounts\" []"));
        assert!(nu_dyn.contains("def \"nu-complete fav medias\" []"));
        assert!(
            nu_dyn
                .contains("^fav __complete sets \"\" | lines | parse \"{value}\\t{description}\"")
        );
        assert!(nu_dyn.contains("@\"nu-complete fav sets\""));
        assert!(nu_dyn.contains("@\"nu-complete fav ups\""));
        assert!(nu_dyn.contains("@\"nu-complete fav accounts\""));
        assert!(nu_dyn.contains("@\"nu-complete fav medias\""));

        let nu_static = generate_completion_script(&mut cmd, CompletionShell::Nushell, "fav", true);
        assert!(!nu_static.contains("nu-complete fav sets"));
        assert!(nu_static.contains("export extern"));

        // Elvish still generates cleanly
        let elvish = generate_completion_script(&mut cmd, CompletionShell::Elvish, "fav", false);
        assert!(!elvish.is_empty());
    }

    #[tokio::test]
    async fn dynamic_complete_medias_filters_before_limit_past_200() {
        use crate::{
            entity::media,
            state::{MediaState, MediaType},
        };

        let db = Db::open_in_memory().await.unwrap();
        let rows: Vec<media::Model> = (1..=260)
            .map(|i| media::Model {
                id: i,
                bv_id: format!("BV1test{i:06}"),
                title: if i == 245 {
                    "Needle Special Target\tWithTab".to_owned()
                } else {
                    format!("Ordinary Video {i}")
                },
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            })
            .collect();
        db.upsert_medias(rows).await.unwrap();

        // Unfiltered still caps cleanly at 200 results
        let all = dynamic_complete_lines(&db, "medias", "").await;
        assert_eq!(all.len(), 200);

        // Filtering finds media #245 (past the 200 cap!) by BVID, AID, and title substring
        let by_bvid = dynamic_complete_lines(&db, "medias", "BV1test000245").await;
        assert_eq!(by_bvid.len(), 1);
        assert_eq!(by_bvid[0], "BV1test000245\tNeedle Special Target WithTab");

        let by_aid = dynamic_complete_lines(&db, "medias", "245").await;
        assert!(by_aid.iter().any(|l| l.starts_with("BV1test000245\t")));

        let by_title = dynamic_complete_lines(&db, "medias", "needle special").await;
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0], "BV1test000245\tNeedle Special Target WithTab");
    }

    #[tokio::test]
    async fn dynamic_complete_lines_strips_nul_and_control_chars() {
        use crate::{
            entity::{account, media, set, up},
            state::{AccountState, MediaState, MediaType, SetKind, SetState, UpState},
        };

        assert_eq!(
            sanitize_completion_desc("A\0B\x1b[31mC\x07\x7f\u{0085}\tD\r\nE"),
            "AB[31mC D  E"
        );

        let db = Db::open_in_memory().await.unwrap();
        db.upsert_sets([set::Model {
            set_id: 1,
            name: "Set\0Bad\x1bTitle\tTab\nNL".to_owned(),
            count: 1,
            state: SetState::Active.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: true,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_ups([up::Model {
            up_id: 2,
            name: "Up\0Name\x07Bell\rCR".to_owned(),
            state: UpState::Active.to_string(),
            sync_complete: true,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_account(account::Model {
            account_id: 3,
            name: "Acct\0User\x7fDel\tTab".to_owned(),
            cookies: String::new(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_medias([media::Model {
            id: 4,
            bv_id: "BV17x411w7KC".to_owned(),
            title: "Vid\0Title\x1bEsc\u{0085}Nel\nLine".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }])
        .await
        .unwrap();

        let sets = dynamic_complete_lines(&db, "sets", "").await;
        assert_eq!(sets, vec!["1\tSetBadTitle Tab NL (Created, Active)"]);

        let ups = dynamic_complete_lines(&db, "ups", "").await;
        assert_eq!(ups, vec!["2\tUpNameBell CR (Active)"]);

        let accounts = dynamic_complete_lines(&db, "accounts", "").await;
        assert_eq!(accounts, vec!["3\tAcctUserDel Tab (Active)"]);

        let medias = dynamic_complete_lines(&db, "medias", "").await;
        assert_eq!(medias, vec!["BV17x411w7KC\tVidTitleEscNel Line"]);
    }

    #[test]
    fn json_flag_works_with_default_format_but_conflicts_with_explicit_format() {
        // `--json` alone succeeds even though `--format` has default_value("table").
        let m = build_command()
            .try_get_matches_from(["fav", "ls", "media", "--json"])
            .unwrap();
        let media_m = m
            .subcommand_matches("list")
            .unwrap()
            .subcommand_matches("media")
            .unwrap();
        assert_eq!(resolve_format(media_m), OutputFormat::Json);

        let m = build_command()
            .try_get_matches_from(["fav", "status", "--json"])
            .unwrap();
        assert_eq!(
            resolve_format(m.subcommand_matches("status").unwrap()),
            OutputFormat::Json
        );

        let m = build_command()
            .try_get_matches_from(["fav", "doctor", "--json"])
            .unwrap();
        assert_eq!(
            resolve_format(m.subcommand_matches("doctor").unwrap()),
            OutputFormat::Json
        );

        let m = build_command()
            .try_get_matches_from(["fav", "auth", "check", "--json"])
            .unwrap();
        let check_m = m
            .subcommand_matches("auth")
            .unwrap()
            .subcommand_matches("check")
            .unwrap();
        assert_eq!(resolve_format(check_m), OutputFormat::Json);

        // Explicit `--format` + `--json` conflicts across all format-supporting commands.
        for args in [
            vec!["fav", "ls", "media", "--format", "tsv", "--json"],
            vec!["fav", "status", "--format", "table", "--json"],
            vec!["fav", "status", "--json", "--format", "json"],
            vec!["fav", "doctor", "--format", "jsonl", "--json"],
            vec!["fav", "auth", "check", "--format", "ids", "--json"],
            vec!["fav", "ls", "--format", "table", "--json"],
        ] {
            let err = build_command()
                .try_get_matches_from(args.clone())
                .expect_err(&format!("expected conflict for {args:?}"));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "expected ArgumentConflict for {args:?}, got: {err}"
            );
        }
    }

    #[test]
    fn runtime_from_matches_tracks_explicit_color_flag() {
        let m_default = build_command()
            .try_get_matches_from(["fav", "status"])
            .unwrap();
        let rt_default = runtime_from_matches(&m_default);
        assert_eq!(rt_default.color, ColorChoice::Auto);
        assert!(!rt_default.color_explicit);

        let m_explicit_before = build_command()
            .try_get_matches_from(["fav", "--color", "auto", "status"])
            .unwrap();
        let rt_explicit_before = runtime_from_matches(&m_explicit_before);
        assert_eq!(rt_explicit_before.color, ColorChoice::Auto);
        assert!(rt_explicit_before.color_explicit);

        let m_explicit_after = build_command()
            .try_get_matches_from(["fav", "status", "--color", "auto"])
            .unwrap();
        let rt_explicit_after = runtime_from_matches(&m_explicit_after);
        assert_eq!(rt_explicit_after.color, ColorChoice::Auto);
        assert!(rt_explicit_after.color_explicit);

        let m_explicit_never = build_command()
            .try_get_matches_from(["fav", "ls", "media", "--color", "never"])
            .unwrap();
        let rt_explicit_never = runtime_from_matches(&m_explicit_never);
        assert_eq!(rt_explicit_never.color, ColorChoice::Never);
        assert!(rt_explicit_never.color_explicit);
    }

    #[test]
    fn config_parent_format_flags_forward_to_subcommands_and_reject_conflicts() {
        use crate::error::ExitKind;

        // Parent `--json` forwards into `show` and `path`.
        let m = build_command()
            .try_get_matches_from(["fav", "config", "--json", "show"])
            .unwrap();
        let cfg_m = m.subcommand_matches("config").unwrap();
        let show_m = cfg_m.subcommand_matches("show").unwrap();
        assert_eq!(
            resolve_subcommand_format(cfg_m, show_m).unwrap(),
            OutputFormat::Json
        );

        let m = build_command()
            .try_get_matches_from(["fav", "config", "--format", "tsv", "path"])
            .unwrap();
        let cfg_m = m.subcommand_matches("config").unwrap();
        let path_m = cfg_m.subcommand_matches("path").unwrap();
        assert_eq!(
            resolve_subcommand_format(cfg_m, path_m).unwrap(),
            OutputFormat::Tsv
        );

        // Child format works when parent did not explicitly specify one.
        let m = build_command()
            .try_get_matches_from(["fav", "config", "show", "--format", "jsonl"])
            .unwrap();
        let cfg_m = m.subcommand_matches("config").unwrap();
        let show_m = cfg_m.subcommand_matches("show").unwrap();
        assert_eq!(
            resolve_subcommand_format(cfg_m, show_m).unwrap(),
            OutputFormat::Jsonl
        );

        // Default when neither parent nor child specifies format.
        let m = build_command()
            .try_get_matches_from(["fav", "config", "path"])
            .unwrap();
        let cfg_m = m.subcommand_matches("config").unwrap();
        let path_m = cfg_m.subcommand_matches("path").unwrap();
        assert_eq!(
            resolve_subcommand_format(cfg_m, path_m).unwrap(),
            OutputFormat::Table
        );

        // Specifying format flags on BOTH parent and child is rejected with Usage error.
        for args in [
            vec!["fav", "config", "--json", "show", "--format", "tsv"],
            vec!["fav", "config", "--format", "tsv", "path", "--json"],
            vec![
                "fav", "config", "--format", "table", "show", "--format", "json",
            ],
            vec!["fav", "config", "--json", "path", "--json"],
        ] {
            let m = build_command().try_get_matches_from(args.clone()).unwrap();
            let cfg_m = m.subcommand_matches("config").unwrap();
            let (_, child_m) = cfg_m.subcommand().unwrap();
            let err = resolve_subcommand_format(cfg_m, child_m)
                .expect_err(&format!("expected conflict error for {args:?}"));
            assert_eq!(err.kind(), ExitKind::Usage);
        }
    }
}
