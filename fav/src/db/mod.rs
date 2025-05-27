pub mod user;

use bevy_ecs::resource::Resource;
use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait as _;

use crate::migration::Migrator;

#[derive(Debug, Clone, Resource)]
pub struct Db {
    db: DatabaseConnection,
}

impl Db {
    pub async fn connect() -> Self {
        std::fs::create_dir_all(".fav").unwrap();
        let db = Database::connect("sqlite://.fav/fav.db?mode=rwc")
            .await
            .unwrap();
        Migrator::up(&db, None).await.unwrap();
        Self { db }
    }
}
