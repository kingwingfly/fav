//! The CLI module.

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;

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
}

#[derive(ValueEnum, Clone)]
enum LoginMethod {
    Password,
    QrCode,
}

#[derive(ValueEnum, Clone)]
enum Kind {
    #[cfg(feature = "bili")]
    Bili,
}

impl Cli {
    /// Run the CLI.
    pub async fn run() {
        let args = Self::parse();
        match args.subcmd {
            Commands::Init { path, kind } => {
                let path = path
                    .unwrap_or(std::path::PathBuf::from("."))
                    .join(".backup");
                std::fs::remove_dir_all(&path).ok();
                std::fs::create_dir_all(&path).unwrap();
                info!("init {}", path.display());
                match kind {
                    #[cfg(feature = "bili")]
                    Kind::Bili => {}
                }
            }
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
            Commands::Fetch {} => todo!(),
        }
    }
}
