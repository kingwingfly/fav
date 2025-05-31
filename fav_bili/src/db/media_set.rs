use anyhow::Result;
use sea_orm::{ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _};

use super::Db;
use crate::entity::media_set;

impl Db {
    pub async fn upsert_media_set(&self, media_set: media_set::Model) -> Result<()> {
        media_set::Entity::insert(media_set.into_active_model())
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await?;
        Ok(())
    }

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

    pub async fn get_media_ids_of_set(&self, set_id: i64) -> Result<Vec<i64>> {
        media_set::Entity::find()
            .filter(media_set::Column::SetId.eq(set_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.id).collect())
    }

    pub async fn get_set_ids_of_media(&self, media_id: i64) -> Result<Vec<i64>> {
        media_set::Entity::find()
            .filter(media_set::Column::Id.eq(media_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.set_id).collect())
    }

    pub async fn delete_media_set(&self, media_set: media_set::Model) -> Result<()> {
        media_set::Entity::delete_by_id((media_set.id, media_set.set_id))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
