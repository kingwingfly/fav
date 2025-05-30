mod account;
mod media;
mod media_set;
mod media_up;
mod set;
mod set_account;

use anyhow::{Context, Result};
use bevy_ecs::resource::Resource;
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait as _;

use crate::migration::Migrator;

#[derive(Debug, Clone, Resource)]
pub struct Db {
    db: DatabaseConnection,
}

impl Db {
    pub async fn connect() -> Result<Self> {
        std::fs::create_dir_all(".fav").context("Failed to create .fav dir")?;
        let db = Database::connect("sqlite://.fav/fav.db?mode=rwc")
            .await
            .context("Failed to connect db")?;
        Migrator::up(&db, None)
            .await
            .context("Failed to update db tables")?;
        Ok(Self { db })
    }
}
