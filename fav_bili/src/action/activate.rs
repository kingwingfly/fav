use anyhow::Result;
use paste::paste;
use tracing::info;

use crate::db::db;

macro_rules! activate {
    ($($obj: ident),+) => {
        $(paste! {
            pub async fn [<activate_ $obj>](id: i64) -> Result<()> {
                let db = db(false).await;
                db.[<activate_ $obj>](id).await?;
                info!(concat!("Activated ", stringify!($obj), "<{}>"), id);
                Ok(())
            }

            pub async fn [<activate _all_ $obj s>]() -> Result<()> {
                let db = db(false).await;
                db.[<activate _all_ $obj s>]().await?;
                info!(concat!("Activated all ", stringify!($obj), "s"));
                Ok(())
            }
        })+
    };
}

activate!(account, set, up);
