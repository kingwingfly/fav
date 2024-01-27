//! The CLI module.
pub(crate) mod utils;

use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::{
    api::{
        auth::{logout, qr_login},
        fetch::fetch,
        init::init,
        like::{like, like_all},
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
        /// Show list status
        #[arg(long, short, group = "l")]
        list: bool,
        /// Show video status
        #[arg(long, short, group = "v")]
        video: bool,
        /// Show tracked only
        #[arg(long, short, requires = "l")]
        tracked: bool,
    },
    /// Track a remote source
    Track {
        /// The id of the source to track
        id: String,
    },
    /// Untrack a remote source
    Untrack {
        /// The id of the source to untrack
        id: String,
    },
    /// Pull remote data
    Pull,
    /// Push local data
    Push,
    /// Like a video
    Like {
        /// The id of the video to like
        id: Option<String>,
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
                AuthCommands::Logout => logout().await.unwrap(),
            },
            Commands::Fetch { prune } => fetch(prune).await.unwrap(),
            Commands::Status {
                list,
                video,
                tracked,
            } => match (list, video) {
                (true, false) => meta().status_list(tracked),
                _ => meta().status_video(),
            },
            Commands::Track { id } => track(id),
            Commands::Untrack { id } => untrack(id),
            Commands::Pull => todo!(),
            Commands::Push => todo!(),
            Commands::Like { id, all } => match (id, all) {
                (Some(id), false) => like(id).await,
                (None, true) => like_all().await,
                (None, false) => Cli::command()
                    .error(ErrorKind::MissingRequiredArgument, "id is required.")
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
