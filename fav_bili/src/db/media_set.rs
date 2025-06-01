use anyhow::Result;
use sea_orm::{EntityTrait as _, IntoActiveModel as _};

use super::Db;
use crate::entity::media_set;

impl Db {
    pub async fn upsert_media_sets(
        &self,
        media_sets: impl IntoIterator<Item = media_set::Model>,
    ) -> Result<()> {
        media_set::Entity::insert_many(media_sets.into_iter().map(|m| m.into_active_model()))
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
