//! The CLI module.
pub(crate) mod utils;

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;

use crate::{
    api::{fetch::fetch, init::init, login::qr_login, track::track, untrack::untrack},
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
    Login {
        /// Login method
        #[arg(value_enum)]
        method: LoginMethod,
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
    /// Ignore
    Ignore,
}

#[derive(ValueEnum, Clone)]
enum LoginMethod {
    Password,
    QrCode,
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
            Commands::Login { method } => match method {
                LoginMethod::Password => {
                    info!("login with password");
                    unimplemented!();
                }
                LoginMethod::QrCode => {
                    info!("login with QR code");
                    qr_login().await.unwrap();
                }
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
            Commands::Ignore => todo!(),
        }
    }
}
