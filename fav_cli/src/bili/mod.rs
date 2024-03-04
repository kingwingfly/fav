//! The CLI for [bilibili](https://www.bilibili.com)

mod action;
use action::*;

use clap::{error::ErrorKind, CommandFactory as _, Parser, Subcommand, ValueHint};
use fav_core::FavCoreResult;

/// The main CLI entry point.
#[derive(Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[clap(subcommand)]
    subcmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the folder for fav
    Init,
    /// Login your account
    Auth {
        /// Login method
        #[clap(subcommand)]
        subcmd: AuthCommands,
    },
    /// Fetch from remote
    Fetch,
    /// Show status of local, default to show resource status
    Status {
        /// Show resource status
        #[arg(value_hint = ValueHint::Other)]
        id: Option<String>,
        /// Show all sets(lists) status
        #[arg(long, short)]
        sets: bool,
        /// Show all resource(video) status
        #[arg(long, short)]
        res: bool,
        /// Show tracked only
        #[arg(long, short)]
        track: bool,
    },
    /// Track a remote source
    Track {
        /// The id of the source to track
        #[arg(value_hint = ValueHint::Other)]
        id: Vec<String>,
    },
    /// Untrack a remote source
    Untrack {
        /// The id of the source to untrack
        #[arg(value_hint = ValueHint::Other)]
        id: Vec<String>,
    },
    /// Pull remote resource to local.  If no id provided, then pull all and skip those having been saved
    Pull {
        /// Optional. The id of the sources to pull forcely or the sets to pull normally
        #[arg(value_hint = ValueHint::Other)]
        id: Option<Vec<String>>,
    },
    /// Interval fetch and pull
    Daemon {
        /// The interval to fetch and pull (in minutes, greater than 15)
        #[arg(value_hint = ValueHint::Other)]
        interval: u64,
    },
    /// Completions for the shell
    Completion {
        /// The shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Login with password
    Login,
    /// Login with QR code
    Logout,
    /// Reuse the login info
    Reuse {
        /// The path of .fav folder, example: /path/to/.fav
        #[arg(value_hint = ValueHint::DirPath)]
        path: std::path::PathBuf,
    },
}

impl Cli {
    /// Run the CLI.
    pub async fn run() -> FavCoreResult<()> {
        let args = Self::parse();
        match args.subcmd {
            Commands::Init => init()?,
            Commands::Auth { subcmd } => match subcmd {
                AuthCommands::Login => login().await?,
                AuthCommands::Logout => logout().await?,
                AuthCommands::Reuse { path } => {
                    std::fs::hard_link(path.join("bili"), ".fav/bili")?;
                }
            },
            Commands::Status {
                id,
                sets,
                res,
                track,
            } => match id {
                Some(id) => {
                    if sets | res | track {
                        Cli::command()
                            .error(
                                ErrorKind::ArgumentConflict,
                                "The id to 'fav status' does not take -s, -r, -t, options.",
                            )
                            .exit();
                    }
                    status(id)?;
                }
                None => match (sets, res) {
                    (false, false) => status_all(sets, true, track)?,
                    _ => status_all(sets, res, track)?,
                },
            },
            Commands::Fetch => fetch().await?,
            Commands::Track { id } => {
                for id in id {
                    track(id)?;
                }
            }
            Commands::Untrack { id } => {
                for id in id {
                    untrack(id)?;
                }
            }
            Commands::Pull { id } => match id {
                Some(id) => {
                    for id in id {
                        pull(id).await?;
                    }
                }
                None => pull_all().await?,
            },
            Commands::Daemon { interval } => daemon(interval).await,
            Commands::Completion { shell } => {
                let mut cmd = Cli::command();
                clap_complete::generate(shell, &mut cmd, "fav", &mut std::io::stdout());
            }
        }
        Ok(())
    }
}
