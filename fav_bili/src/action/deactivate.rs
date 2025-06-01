use anyhow::Result;
use paste::paste;
use tracing::info;

use crate::db::db;

macro_rules! deactivate {
    ($($obj: ident),+) => {
        $(paste! {
            pub async fn [<deactivate_ $obj>](account_id: i64) -> Result<()> {
                let db = db().await;
                db.[<deactivate_ $obj>](account_id).await?;
                info!(concat!("Deactivated ", stringify!($obj), "<{}>"), account_id);
                Ok(())
            }

            pub async fn [<deactivate _all_ $obj s>]() -> Result<()> {
                let db = db().await;
                db.[<deactivate _all_ $obj s>]().await?;
                info!(concat!("Deactivated all ", stringify!($obj), "s"));
                Ok(())
            }
        })+
    };
}

deactivate!(account, set, up);
