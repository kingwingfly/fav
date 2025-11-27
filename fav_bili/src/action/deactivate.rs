use anyhow::Result;
use paste::paste;
use tracing::info;

use crate::db::db;

macro_rules! deactivate {
    ($($obj: ident),+) => {
        $(paste! {
            pub async fn [<deactivate_ $obj>](id: i64) -> Result<()> {
                let db = db(false).await;
                db.[<deactivate_ $obj>](id).await?;
                info!(concat!("Deactivated ", stringify!($obj), "<{}>"), id);
                Ok(())
            }

            pub async fn [<deactivate _all_ $obj s>]() -> Result<()> {
                let db = db(false).await;
                db.[<deactivate _all_ $obj s>]().await?;
                info!(concat!("Deactivated all ", stringify!($obj), "s"));
                Ok(())
            }
        })+
    };
}

deactivate!(account, set, up);
