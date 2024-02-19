use fav_cli::bili::Cli;
use tracing::{error, Level};
use tracing_subscriber::{filter, prelude::*};

#[tokio::main]
async fn main() {
    let filter = filter::Targets::new()
        // Enable the `INFO` level for anything in `fav`
        .with_target("fav_core", Level::INFO)
        .with_target("fav_utils", Level::INFO)
        .with_target("fav", Level::INFO);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_target(false),
        )
        .with(filter)
        .init();

    if let Err(e) = Cli::run().await {
        error!("{e}");
    }
}
