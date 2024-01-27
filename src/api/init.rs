use super::error::Result;
use crate::{api::auth::logout, cli::Kind};
use std::path::PathBuf;
use tracing::info;

pub(crate) async fn init(path: Option<PathBuf>, kind: Kind) -> Result<()> {
    let path = path
        .unwrap_or(std::path::PathBuf::from("."))
        .join(".backup");
    if path.join("cookie").exists() {
        info!("Try logging out");
        logout().await?;
    }
    if path.exists() {
        info!("remove old {}", path.display());
        std::fs::remove_dir_all(&path).ok();
    }
    std::fs::create_dir_all(&path).unwrap();
    info!("init {}", path.display());
    match kind {
        #[cfg(feature = "bili")]
        Kind::Bili => std::fs::write(path.join("kind"), "bili").unwrap(),
    }
    Ok(())
}
