//! The CLI module.
pub(crate) mod utils;

use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::{
    api::{
        auth::{logout, qr_login},
        fetch::fetch,
        init::init,
        like::{like, like_all},
        pull::{pull, pull_all},
        track::track,
        untrack::untrack,
    },
    meta::meta,
};

/// The main CLI entry point.
#[derive(Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[clap(subcommand)]
    subcmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the folder for backup
    Init {
        #[arg(value_enum)]
        kind: Kind,
        /// The path to store the backup
        path: Option<std::path::PathBuf>,
    },
    /// Login your account
    Auth {
        /// Login method
        #[clap(subcommand)]
        subcmd: AuthCommands,
    },
    /// Fetch from remote
    Fetch {
        /// Prune data no longer on remote
        #[arg(long, short)]
        prune: bool,
    },
    /// Show status of local, default to show video status
    Status {
        /// Show resource status
        id: Option<String>,
        /// Show all list status
        #[arg(long, short)]
        list: bool,
        /// Show all video status
        #[arg(long, short)]
        video: bool,
        /// Show tracked only
        #[arg(long, short)]
        tracked: bool,
    },
    /// Track a remote source
    Track {
        /// The id of the source to track
        id: Vec<String>,
    },
    /// Untrack a remote source
    Untrack {
        /// The id of the source to untrack
        id: Vec<String>,
    },
    /// Pull remote data
    Pull {
        /// The id of the source to pull
        id: Option<Vec<String>>,
    },
    /// Push local data
    Push,
    /// Like a video
    Like {
        /// The id of the video to like
        bvid: Option<Vec<String>>,
        /// Like all videos tracked
        #[arg(long, short)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Login with password
    Login,
    /// Login with QR code
    Logout,
}

#[derive(ValueEnum, Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum Kind {
    #[cfg(feature = "bili")]
    Bili,
}

impl Cli {
    /// Run the CLI.
    pub async fn run() {
        let args = Self::parse();
        match args.subcmd {
            Commands::Init { path, kind } => init(path, kind).await.unwrap(),
            Commands::Auth { subcmd } => match subcmd {
                AuthCommands::Login => qr_login().await.unwrap(),
                AuthCommands::Logout => logout().await,
            },
            Commands::Fetch { prune } => fetch(prune).await.unwrap(),
            Commands::Status {
                id,
                list,
                video,
                tracked,
            } => match (id, list, video) {
                (Some(id), false, false) => meta().status_of(id),
                (None, true, false) => meta().status_list(tracked),
                (Some(_), list, video) if (list | video) => Cli::command()
                    .error(
                        ErrorKind::ArgumentConflict,
                        "The -l, -v options to 'backup status' does not take a id.",
                    )
                    .exit(),
                _ => meta().status_video(tracked),
            },
            Commands::Track { id } => id.into_iter().for_each(track),
            Commands::Untrack { id } => id.into_iter().for_each(untrack),
            Commands::Pull { id } => match id {
                Some(id) => pull(id).await,
                None => pull_all().await,
            },
            Commands::Push => todo!(),
            Commands::Like { bvid, all } => match (bvid, all) {
                (Some(bvid), false) => like(bvid).await,
                (None, true) => like_all().await,
                (None, false) => Cli::command()
                    .error(ErrorKind::MissingRequiredArgument, "bvid is required.")
                    .exit(),
                (Some(_), true) => Cli::command()
                    .error(
                        ErrorKind::ArgumentConflict,
                        "The -a, options to 'git branch' does not take a id.",
                    )
                    .exit(),
            },
        }
    }
}
