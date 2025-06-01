mod action;
mod api;
mod command;
mod cookies;
mod db;
mod entity;
mod migration;
mod payload;
mod response;
mod state;
mod table;
mod version;
mod wbi;

use command::FavCommand;

use tracing::error;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = FavCommand::new().run().await {
        error!("{}", e);
    }
}
