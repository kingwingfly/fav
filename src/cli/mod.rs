//! The CLI module.

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;

use crate::api::{fetch::fetch, init::init, status::status};

/// The main CLI entry point.
#[derive(Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[clap(subcommand)]
    subcmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(value_enum)]
        kind: Kind,
        path: Option<std::path::PathBuf>,
    },
    Login {
        #[arg(value_enum)]
        method: LoginMethod,
    },
    Fetch {},
    Status {},
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
                    crate::api::login::qr_login().await.unwrap();
                }
            },
            Commands::Fetch {} => fetch().await.unwrap(),
            Commands::Status {} => status().unwrap(),
        }
    }
}
