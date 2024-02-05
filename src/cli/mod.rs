//! The CLI module.
pub(crate) mod utils;

use clap::{
    error::ErrorKind, Command, CommandFactory, FromArgMatches as _, Parser, Subcommand, ValueEnum,
    ValueHint,
};

use crate::{
    api::{
        auth::{logout, qr_login},
        fetch::fetch,
        init::init,
        like::{like, like_all},
        modify::modify,
        pull::{pull, pull_all},
        track::track,
        untrack::untrack,
    },
    config::set_ffmpeg_path,
    proto::data::{Meta, Qn},
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
    /// Initialize the folder for fav
    Init {
        #[arg(value_enum, value_parser = ["bili"])]
        kind: Kind,
        /// The path to store the fav
        #[arg(value_hint = ValueHint::DirPath)]
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
        #[arg(value_hint = ValueHint::Other)]
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
        #[arg(value_hint = ValueHint::Other)]
        id: Vec<String>,
    },
    /// Untrack a remote source
    Untrack {
        /// The id of the source to untrack
        #[arg(value_hint = ValueHint::Other)]
        id: Vec<String>,
    },
    /// Pull remote data
    Pull {
        /// The id of the source to pull
        #[arg(value_hint = ValueHint::Other)]
        id: Option<Vec<String>>,
    },
    /// Push local data
    Push,
    /// Like a video
    Like {
        /// The id of the video to like
        #[arg(value_hint = ValueHint::Other)]
        bvid: Option<Vec<String>>,
        /// Like all videos tracked
        #[arg(long, short)]
        all: bool,
    },
    /// Set the path of ffmpeg
    Ffmpeg {
        /// Set the path of ffmpeg
        #[arg(value_hint = ValueHint::FilePath)]
        path: String,
    },
    /// Interval fetch and pull
    Daemon {
        /// The interval to fetch and pull (in minutes, greater than 15)
        #[arg(value_hint = ValueHint::Other)]
        interval: u64,
    },
    /// Modify resource status
    Modify {
        /// The id of the resources to modify
        #[arg(value_hint = ValueHint::Other)]
        id: Vec<String>,
        /// Mark saved true or false
        #[arg(long, short, value_parser = ["true", "false"])]
        saved: Option<bool>,
        /// modify the clarity
        #[arg(long, short, value_enum)]
        clarity: Option<Qn>,
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
pub(crate) enum Kind {
    #[cfg(feature = "bili")]
    Bili,
}

impl Cli {
    fn build_cli() -> Command {
        clap_complete::dynamic::shells::CompleteCommand::augment_subcommands(Self::command())
    }

    /// Run the CLI.
    pub async fn run() {
        let cmd = Self::build_cli();
        let matches = cmd.get_matches();
        if let Ok(completions) =
            clap_complete::dynamic::shells::CompleteCommand::from_arg_matches(&matches)
        {
            completions.try_complete(&mut Self::build_cli()).unwrap();
            return;
        }

        let args = Self::parse();
        match args.subcmd {
            Commands::Init { path, kind } => init(path, kind).await.unwrap(),
            Commands::Auth { subcmd } => match subcmd {
                AuthCommands::Login => qr_login().await.unwrap(),
                AuthCommands::Logout => logout().await,
            },
            Commands::Fetch { prune } => fetch(prune, true).await.unwrap(),
            Commands::Status {
                id,
                list,
                video,
                tracked,
            } => match (id, list, video) {
                (Some(id), false, false) => Meta::read().status_of(id),
                (None, true, false) => Meta::read().status_list(tracked),
                (Some(_), list, video) if (list | video) => Cli::command()
                    .error(
                        ErrorKind::ArgumentConflict,
                        "The -l, -v options to 'fav status' does not take a id.",
                    )
                    .exit(),
                _ => Meta::read().status_video(tracked),
            },
            Commands::Track { id } => track(id),
            Commands::Untrack { id } => untrack(id),
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
            Commands::Ffmpeg { path } => set_ffmpeg_path(path).await,
            Commands::Daemon { interval } => crate::daemon::interval(interval).await,
            Commands::Modify { id, saved, clarity } => modify(id, saved, clarity),
        }
    }
}
