use clap::{Parser, Subcommand};
use tracing::info;

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[clap(subcommand)]
    subcmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init { path: Option<std::path::PathBuf> },
    Fetch {},
}

fn main() {
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let args = Cli::parse();
    match args.subcmd {
        Commands::Init { path } => {
            let path = path
                .unwrap_or(std::path::PathBuf::from("."))
                .join(".backup");
            info!("init {}", path.display());
            std::fs::create_dir_all(&path).unwrap();
        }
        Commands::Fetch {} => todo!(),
    }
}
