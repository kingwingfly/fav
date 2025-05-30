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

fn main() {
    if let Err(e) = FavCommand::new().run() {
        error!("{}", e);
    }
}
