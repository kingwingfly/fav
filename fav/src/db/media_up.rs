use anyhow::Result;
use sea_orm::{ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _};

use super::Db;
use crate::entity::media_up;

impl Db {
    pub async fn upsert_media_up(&self, media_up: media_up::Model) -> Result<()> {
        media_up::Entity::insert(media_up.into_active_model())
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await?;
        Ok(())
    }

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

    pub async fn get_media_ids_of_up(&self, up_id: i64) -> Result<Vec<i64>> {
        media_up::Entity::find()
            .filter(media_up::Column::UpId.eq(up_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.id).collect())
    }

    pub async fn get_up_ids_of_media(&self, media_id: i64) -> Result<Vec<i64>> {
        media_up::Entity::find()
            .filter(media_up::Column::Id.eq(media_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.up_id).collect())
    }

    pub async fn delete_media_up(&self, media_up: media_up::Model) -> Result<()> {
        media_up::Entity::delete_by_id((media_up.id, media_up.up_id))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
