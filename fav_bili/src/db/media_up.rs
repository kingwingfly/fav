use anyhow::Result;
use sea_orm::{EntityTrait as _, IntoActiveModel as _};

use super::Db;
use crate::entity::media_up;

impl Db {
    pub async fn upsert_media_ups(
        &self,
        media_ups: impl IntoIterator<Item = media_up::Model>,
    ) -> Result<()> {
        media_up::Entity::insert_many(media_ups.into_iter().map(|m| m.into_active_model()))
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
