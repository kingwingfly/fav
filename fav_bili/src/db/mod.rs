mod account;
mod media;
mod media_set;
mod media_up;
mod set;
mod set_account;
mod up;
mod up_account;

use std::process::exit;

use anyhow::{Context, Result};
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait as _;
use tokio::sync::OnceCell;
use tracing::error;

use crate::migration::Migrator;

static DB: OnceCell<Db> = OnceCell::const_new();

pub async fn db(create: bool) -> &'static Db {
    DB.get_or_init(async move || match Db::connect(create).await {
        Ok(db) => db,
        Err(e) => {
            error!("Login first, no db yet; {}", e);
            exit(-1)
        }
    })
    .await
}

#[derive(Debug, Clone)]
pub struct Db {
    db: DatabaseConnection,
}

impl Db {
    pub async fn connect(create: bool) -> Result<Self> {
        if create {
            std::fs::create_dir_all(".fav").context("Failed to create .fav dir")?;
        }
        let db = Database::connect(if create {
            "sqlite://.fav/fav.db?mode=rwc"
        } else {
            "sqlite://.fav/fav.db?mode=rw"
        })
        .await
        .context("Failed to connect db")?;
        Migrator::up(&db, None)
            .await
            .context("Failed to update db tables")?;
        Ok(Self { db })
    }
}
