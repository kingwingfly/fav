use std::{
    io::{self, Write as _},
    path::PathBuf,
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
        deactivate::{DeactivateOptions, deactivate},
        doctor::doctor,
        fetch::{FetchOptions, fetch},
        like::{LikeOptions, like},
        list::{ListKind, ListOptions, list},
        media::{MediaResetOptions, reset as media_reset, skip as media_skip},
        pull::{PullOptions, pull},
        schedule::{ScheduleTarget, schedule},
        status::status,
        sync::{SyncOptions, sync},
    },
    config::{Workspace, parse_rate_limit},
    context::{AppContext, RuntimeOptions},
    error::{AppError, AppResult},
    table::{ColorChoice, OutputFormat, should_color},
    version::VERSION,
};

const COMPLETION_AFTER_HELP: &str = "\
Examples:
  bash:       fav completion bash > ~/.local/share/bash-completion/completions/fav
  zsh:        fav completion zsh > ~/.zfunc/_fav
  fish:       fav completion fish > ~/.config/fish/completions/fav.fish
  powershell: fav completion powershell | Out-String | Invoke-Expression";

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
            .action(ArgAction::SetTrue),
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
            .help("Disable incremental early-stop and scan all pages")
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
                            args.push(
                                Arg::new("account")
                                    .long("account")
                                    .help("Filter sets belonging to account ID")
                                    .value_parser(value_parser!(i64)),
                            );
                            args
                        }),
                    Command::new("up")
                        .about("List UPs [alias: upper, u]")
                        .aliases(["upper", "u"])
                        .args({
                            let mut args = list_common_args();
                            args.push(
                                Arg::new("account")
                                    .long("account")
                                    .help("Filter UPs followed by account ID")
                                    .value_parser(value_parser!(i64)),
                            );
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
                .about("Download and mux pending medias [alias: p]")
                .aliases(["p"])
                .args({
                    let mut args = vec![
                        Arg::new("targets")
                            .help("Specific BVID(s), avID(s), numeric AID(s), or Bilibili video URL(s) to pull")
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
            Command::new("like")
                .about("Like (or unlike) Bilibili medias by BVID/AID/URL, set, or stdin")
                .args([
                    Arg::new("targets")
                        .help("BVID(s), avID(s), numeric AID(s), URL(s), or '-' to read from stdin")
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
                    args.push(
                        Arg::new("offline")
                            .long("offline")
                            .help("Skip live network authentication checks")
                            .action(ArgAction::SetTrue),
                    );
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
                                .help("Reset Completed medias whose downloaded files are missing on disk")
                                .action(ArgAction::SetTrue),
                            Arg::new("all")
                                .long("all")
                                .short('a')
                                .help("Reset all tracked medias")
                                .action(ArgAction::SetTrue),
                            Arg::new("dry-run")
                                .long("dry-run")
                                .help("Preview how many medias would be reset")
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
                    Arg::new("bin")
                        .long("bin")
                        .help("Path to the `fav` binary to invoke")
                        .value_parser(value_parser!(PathBuf)),
                    Arg::new("install")
                        .long("install")
                        .help("Write user-level schedule files directly (no sudo)")
                        .action(ArgAction::SetTrue),
                ]),
            Command::new("completion")
                .about("Generate shell completion scripts (with optional dynamic DB completion)")
                .after_help(COMPLETION_AFTER_HELP)
                .arg_required_else_help(true)
                .args([
                    Arg::new("shell")
                        .help("Target shell")
                        .required(true)
                        .value_parser(value_parser!(Shell)),
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
        set_id: matches.try_get_one::<i64>("set").ok().flatten().copied(),
        up_id: matches.try_get_one::<i64>("up").ok().flatten().copied(),
        account_id: matches
            .try_get_one::<i64>("account")
            .ok()
            .flatten()
            .copied(),
        active_only: matches.get_flag("active"),
        inactive_only: matches.get_flag("inactive"),
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

fn dynamic_completion_snippet(shell: Shell, bin_name: &str) -> String {
    let fn_ident = sanitize_shell_fn_ident(bin_name);
    match shell {
        Shell::Fish => format!(
            "\n# Dynamic workspace DB completions via `{bin_name} __complete`\n\
             complete -c {bin_name} -n '__fish_seen_subcommand_from activate active a deactivate deactive d; and __fish_seen_subcommand_from set s list collection l c' -f -a '({bin_name} __complete sets (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -n '__fish_seen_subcommand_from activate active a deactivate deactive d; and __fish_seen_subcommand_from up upper u' -f -a '({bin_name} __complete ups (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -n '__fish_seen_subcommand_from activate active a deactivate deactive d; and __fish_seen_subcommand_from account a' -f -a '({bin_name} __complete accounts (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -n '__fish_seen_subcommand_from auth; and __fish_seen_subcommand_from logout check' -f -a '({bin_name} __complete accounts (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -n '__fish_seen_subcommand_from pull p like' -f -a '({bin_name} __complete medias (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -n '__fish_seen_subcommand_from media; and __fish_seen_subcommand_from reset skip' -f -a '({bin_name} __complete medias (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -l set -r -f -a '({bin_name} __complete sets (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -l up -r -f -a '({bin_name} __complete ups (commandline -ct) 2>/dev/null)'\n\
             complete -c {bin_name} -l account -r -f -a '({bin_name} __complete accounts (commandline -ct) 2>/dev/null)'\n"
        ),
        Shell::Bash => format!(
            "\n# Dynamic workspace DB completions via `{bin_name} __complete`\n\
             _{fn_ident}_dynamic_candidates() {{\n\
                 local kind=\"$1\" cur=\"$2\"\n\
                 COMPREPLY=( $(compgen -W \"$({bin_name} __complete \"$kind\" \"$cur\" 2>/dev/null | cut -f1)\" -- \"$cur\") )\n\
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
                 case \"${{prev}}\" in\n\
                     --set)\n\
                         _{fn_ident}_dynamic_candidates sets \"${{cur}}\"\n\
                         return 0\n\
                         ;;\n\
                     --up)\n\
                         _{fn_ident}_dynamic_candidates ups \"${{cur}}\"\n\
                         return 0\n\
                         ;;\n\
                     --account)\n\
                         _{fn_ident}_dynamic_candidates accounts \"${{cur}}\"\n\
                         return 0\n\
                         ;;\n\
                 esac\n\
                 local sub1=\"\" sub2=\"\" i=1 w\n\
                 while [[ $i -lt $COMP_CWORD ]]; do\n\
                     w=\"${{COMP_WORDS[$i]}}\"\n\
                     case \"$w\" in\n\
                         -C|--dir|--color|--format|--state|--search|--sort|--limit|--type|--set|--up|--account|-j|--jobs|-o|--output|--limit-rate|--top|--target|--interval|--bin|--bin-name|--file)\n\
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
                         _{fn_ident}_dynamic_candidates sets \"${{cur}}\"\n\
                         return 0\n\
                         ;;\n\
                     activate:up|activate:upper|activate:u|active:up|active:upper|active:u|a:up|a:upper|a:u|\\\n\
                     deactivate:up|deactivate:upper|deactivate:u|deactive:up|deactive:upper|deactive:u|d:up|d:upper|d:u)\n\
                         _{fn_ident}_dynamic_candidates ups \"${{cur}}\"\n\
                         return 0\n\
                         ;;\n\
                     activate:account|activate:a|active:account|active:a|a:account|a:a|\\\n\
                     deactivate:account|deactivate:a|deactive:account|deactive:a|d:account|d:a|\\\n\
                     auth:logout|auth:check)\n\
                         _{fn_ident}_dynamic_candidates accounts \"${{cur}}\"\n\
                         return 0\n\
                         ;;\n\
                     pull:*|p:*|like:*|media:reset|media:skip)\n\
                         _{fn_ident}_dynamic_candidates medias \"${{cur}}\"\n\
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
        Shell::Zsh => format!(
            "\n# Dynamic workspace DB completions via `{bin_name} __complete`\n\
             _{fn_ident}_complete_db() {{\n\
                 local kind=\"$1\" prefix=\"$2\"\n\
                 local -a entries\n\
                 local line id desc\n\
                 while IFS=$'\\t' read -r id desc; do\n\
                     [[ -n \"$id\" ]] || continue\n\
                     desc=\"${{desc//:/\\\\:}}\"\n\
                     entries+=(\"${{id}}:${{desc}}\")\n\
                 done < <({bin_name} __complete \"$kind\" \"$prefix\" 2>/dev/null)\n\
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
                 case \"$prev\" in\n\
                     --set)\n\
                         _{fn_ident}_complete_db sets \"$cur\"\n\
                         return $?\n\
                         ;;\n\
                     --up)\n\
                         _{fn_ident}_complete_db ups \"$cur\"\n\
                         return $?\n\
                         ;;\n\
                     --account)\n\
                         _{fn_ident}_complete_db accounts \"$cur\"\n\
                         return $?\n\
                         ;;\n\
                 esac\n\
                 local sub1=\"\" sub2=\"\" i=2 w\n\
                 while (( i < CURRENT )); do\n\
                     w=\"${{words[i]}}\"\n\
                     case \"$w\" in\n\
                         -C|--dir|--color|--format|--state|--search|--sort|--limit|--type|--set|--up|--account|-j|--jobs|-o|--output|--limit-rate|--top|--target|--interval|--bin|--bin-name|--file)\n\
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
                         _{fn_ident}_complete_db sets \"$cur\"\n\
                         return $?\n\
                         ;;\n\
                     activate:up|activate:upper|activate:u|active:up|active:upper|active:u|a:up|a:upper|a:u|\\\n\
                     deactivate:up|deactivate:upper|deactivate:u|deactive:up|deactive:upper|deactive:u|d:up|d:upper|d:u)\n\
                         _{fn_ident}_complete_db ups \"$cur\"\n\
                         return $?\n\
                         ;;\n\
                     activate:account|activate:a|active:account|active:a|a:account|a:a|\\\n\
                     deactivate:account|deactivate:a|deactive:account|deactive:a|d:account|d:a|\\\n\
                     auth:logout|auth:check)\n\
                         _{fn_ident}_complete_db accounts \"$cur\"\n\
                         return $?\n\
                         ;;\n\
                     pull:*|p:*|like:*|media:reset|media:skip)\n\
                         _{fn_ident}_complete_db medias \"$cur\"\n\
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
        Shell::PowerShell => format!(
            "\n# Dynamic workspace DB helper: `{bin_name} __complete <sets|ups|accounts|medias> [prefix]`\n"
        ),
        _ => String::new(),
    }
}

fn generate_completion_script(
    cmd: &mut Command,
    shell: Shell,
    bin_name: &str,
    static_only: bool,
) -> String {
    let mut buf = Vec::new();
    clap_complete::generate(shell, cmd, bin_name, &mut buf);
    let mut script = String::from_utf8(buf).unwrap_or_default();
    if !static_only {
        if shell == Shell::Zsh {
            let fn_ident = sanitize_shell_fn_ident(bin_name);
            let clap_tail = format!(
                "\nif [ \"$funcstack[1]\" = \"_{fn_ident}\" ]; then\n    _{fn_ident} \"$@\"\nelse\n    compdef _{fn_ident} {bin_name}\nfi\n"
            );
            if let Some(stripped) = script.strip_suffix(&clap_tail) {
                script = stripped.to_owned();
            }
        }
        let snippet = dynamic_completion_snippet(shell, bin_name);
        if !snippet.is_empty() {
            script.push_str(&snippet);
        }
    }
    script
}

async fn run_dynamic_complete(runtime: RuntimeOptions, kind: &str, prefix: &str) -> AppResult<()> {
    let Ok(ctx) = AppContext::open_read_only(runtime).await else {
        return Ok(());
    };
    let needle = prefix.trim().to_ascii_lowercase();
    let matches_prefix = |id: &str, label: &str| {
        needle.is_empty()
            || id.to_ascii_lowercase().starts_with(&needle)
            || label.to_ascii_lowercase().contains(&needle)
    };

    match kind {
        "sets" => {
            if let Ok(rows) = ctx.db.all_sets().await {
                for s in rows {
                    let id = s.set_id.to_string();
                    if matches_prefix(&id, &s.name) {
                        println!("{id}\t{} ({})", s.name.replace('\t', " "), s.state);
                    }
                }
            }
        }
        "ups" => {
            if let Ok(rows) = ctx.db.all_ups().await {
                for u in rows {
                    let id = u.up_id.to_string();
                    if matches_prefix(&id, &u.name) {
                        println!("{id}\t{} ({})", u.name.replace('\t', " "), u.state);
                    }
                }
            }
        }
        "accounts" => {
            if let Ok(rows) = ctx.db.all_accounts().await {
                for a in rows {
                    let id = a.account_id.to_string();
                    if matches_prefix(&id, &a.name) {
                        println!("{id}\t{} ({})", a.name.replace('\t', " "), a.state);
                    }
                }
            }
        }
        "medias" => {
            if let Ok(rows) = ctx.db.all_medias().await {
                for m in rows.into_iter().take(200) {
                    if matches_prefix(&m.bv_id, &m.title)
                        || matches_prefix(&m.id.to_string(), &m.title)
                    {
                        println!("{}\t{}", m.bv_id, m.title.replace('\t', " "));
                    }
                }
            }
        }
        _ => {}
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
                .get_one::<Shell>("shell")
                .ok_or_else(|| AppError::usage(anyhow!("missing required <shell> argument")))?;
            let bin_name = sub
                .get_one::<String>("bin-name")
                .cloned()
                .unwrap_or_else(|| "fav".to_owned());
            let static_only = sub.get_flag("static");

            let script = generate_completion_script(cmd, shell, &bin_name, static_only);
            let mut stdout = io::stdout().lock();
            let _ = stdout.write_all(script.as_bytes());
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
            let install = sub.get_flag("install");
            let ws = Workspace::discover_or_init(runtime.explicit_dir.as_deref())?;
            let ws_root =
                std::fs::canonicalize(ws.root()).unwrap_or_else(|_| ws.root().to_path_buf());
            let fav_bin = sub
                .get_one::<PathBuf>("bin")
                .cloned()
                .or_else(|| std::env::current_exe().ok())
                .unwrap_or_else(|| PathBuf::from("fav"));
            let output =
                schedule(target, interval, &ws_root, &fav_bin, install).map_err(AppError::usage)?;
            println!("{output}");
            Ok(())
        }
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
            doctor(&ctx, resolve_format(sub), sub.get_flag("offline")).await
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
                            all: s.get_flag("all"),
                            dry_run: s.get_flag("dry-run"),
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
    fn completion_scripts_wire_dynamic_wrappers_and_respect_static_flag() {
        let mut cmd = build_command();
        let bash_dyn = generate_completion_script(&mut cmd, Shell::Bash, "fav", false);
        assert!(bash_dyn.contains("_fav_with_dynamic()"));
        assert!(bash_dyn.contains("complete -F _fav_with_dynamic"));
        assert!(bash_dyn.contains("fav __complete"));

        let bash_static = generate_completion_script(&mut cmd, Shell::Bash, "fav", true);
        assert!(!bash_static.contains("_fav_with_dynamic"));

        let zsh_dyn = generate_completion_script(&mut cmd, Shell::Zsh, "fav", false);
        assert!(zsh_dyn.contains("_fav_with_dynamic()"));
        assert!(zsh_dyn.contains("compdef _fav_with_dynamic fav"));
        assert!(!zsh_dyn.contains("compdef _fav fav"));

        let zsh_static = generate_completion_script(&mut cmd, Shell::Zsh, "fav", true);
        assert!(!zsh_static.contains("_fav_with_dynamic"));
        assert!(zsh_static.contains("compdef _fav fav"));

        let fish_dyn = generate_completion_script(&mut cmd, Shell::Fish, "fav", false);
        assert!(fish_dyn.contains("fav __complete sets"));
        assert!(fish_dyn.contains("-l set"));
    }
}
