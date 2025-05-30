mod api;
mod command;
mod cookies;
mod db;
mod entity;
mod event;
mod migration;
mod payload;
mod response;
mod runtime;
mod state;
mod system;
mod table;
mod version;
mod wbi;

use command::FavCommand;

use tracing::error;
use tracing_subscriber::EnvFilter;

fn main() {
    let filter = EnvFilter::from_default_env().add_directive("fav=info".parse().unwrap());
    #[cfg(debug_assertions)]
    let filter = filter.add_directive("reqwest=debug".parse().unwrap());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stdout)
        .with_line_number(true)
        .init();
    if let Err(e) = FavCommand::new().run() {
        error!("{}", e);
    }
}
