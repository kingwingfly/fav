//! This is the build script for the project.
//! - It generates the protobuf file for the project.
//! - It generates the completion file for the CLI.
//! Caution:
//! You should copy `Cli` relevant code from `src/cli/mod.rs` to here and `Qn` from `src/proto/data`.
//! So that the completion file can be generated correctly.

use clap::{builder::PossibleValue, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate_to, Shell};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    protobuf_codegen::Codegen::new()
        .pure()
        .includes(["./proto"])
        .inputs(["./proto/data.proto"])
        .out_dir("./src/proto")
        .run()
        .unwrap();

    if let Some(outdir) = std::env::var_os("OUT_DIR") {
        let mut cmd = Cli::command();
        for &shell in Shell::value_variants() {
            let path = generate_to(
                shell, &mut cmd, // We need to specify what generator to use
                "fav",    // We need to specify the bin name manually
                &outdir,  // We need to specify where to write to
            )?;
            let filename = path.file_name().unwrap();
            let release_path = std::path::PathBuf::from(&outdir)
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(filename);
            std::fs::rename(&path, &release_path).unwrap();
            println!("cargo:warning=Completion file is generated: {release_path:?}");
        }
    }
    Ok(())
}

/// The main CLI entry point.
#[derive(Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[clap(subcommand)]
    pub(crate) subcmd: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Initialize the folder for fav
    Init {
        #[arg(value_enum)]
        kind: Kind,
        /// The path to store the fav
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
    /// Set the path of ffmpeg
    Ffmpeg {
        /// Set the path of ffmpeg
        path: String,
    },
    /// Interval fetch and pull
    Daemon {
        /// The interval to fetch and pull (in minutes, greater than 15)
        interval: u64,
    },
    /// Modify resource status
    Modify {
        /// The id of the resources to modify
        id: Vec<String>,
        /// Mark saved true or false
        #[arg(long, short)]
        saved: Option<bool>,
        /// modify the clarity
        #[arg(long, short, value_enum)]
        clarity: Option<Qn>,
    },
}

#[derive(Subcommand)]
pub(crate) enum AuthCommands {
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

#[derive(Clone)]
enum Qn {
    Default = 0,
    EightK = 127,
    Dolby = 126,
    HDR = 125,
    FourK = 120,
    FullHDHighFrame = 116,
    FullHDHighCode = 112,
    FullHD = 80,
    HDHighFrame = 74,
    HD = 64,
    SD = 32,
    LD = 16,
    VLD = 6,
}

impl ValueEnum for Qn {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::Default,
            Self::EightK,
            Self::Dolby,
            Self::HDR,
            Self::FourK,
            Self::FullHDHighFrame,
            Self::FullHDHighCode,
            Self::FullHD,
            Self::HDHighFrame,
            Self::HD,
            Self::SD,
            Self::LD,
            Self::VLD,
        ]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            Qn::Default => Some(PossibleValue::new("default")),
            Qn::EightK => Some(PossibleValue::new("8k")),
            Qn::Dolby => Some(PossibleValue::new("dolby")),
            Qn::HDR => Some(PossibleValue::new("hdr")),
            Qn::FourK => Some(PossibleValue::new("4k")),
            Qn::FullHDHighFrame => Some(PossibleValue::new("1080p60")),
            Qn::FullHDHighCode => Some(PossibleValue::new("1080p+")),
            Qn::FullHD => Some(PossibleValue::new("1080p")),
            Qn::HDHighFrame => Some(PossibleValue::new("720p60")),
            Qn::HD => Some(PossibleValue::new("720p")),
            Qn::SD => Some(PossibleValue::new("480p")),
            Qn::LD => Some(PossibleValue::new("360p")),
            Qn::VLD => Some(PossibleValue::new("240p")),
        }
    }
}
