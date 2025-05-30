use anyhow::{Context as _, Result};
use fav::migration::OnConflict;
use sea_orm::{
    ActiveValue::{Set, Unchanged},
    ColumnTrait, EntityTrait as _, IntoActiveModel as _, QueryFilter,
};

use super::Db;
use crate::{entity::media, state::MediaState};

impl Db {
    pub async fn upsert_media(&self, media: media::Model) -> Result<()> {
        media::Entity::insert(media.into_active_model())
            .on_conflict(
                OnConflict::column(media::Column::BvId)
                    .update_columns([media::Column::Title, media::Column::Id, media::Column::Type])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn upsert_medias(
        &self,
        medias: impl IntoIterator<Item = media::Model>,
    ) -> Result<()> {
        media::Entity::insert_many(medias.into_iter().map(|m| m.into_active_model()))
            .on_conflict(
                OnConflict::column(media::Column::BvId)
                    .update_columns([media::Column::Title, media::Column::Id, media::Column::Type])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_media(&self, id: i64) -> Result<media::Model> {
        media::Entity::find_by_id(id.to_owned())
            .one(&self.db)
            .await?
            .context(format!("Unknown media<{}>", id))
    }

    pub async fn delete_media(&self, id: i64) -> Result<()> {
        media::Entity::delete_by_id(id).exec(&self.db).await?;
        Ok(())
    }

    pub async fn set_media_state(&self, id: i64, state: MediaState) -> Result<()> {
        media::Entity::update(media::ActiveModel {
            id: Unchanged(id),
            state: Set(state.to_string()),
            ..Default::default()
        });
        Ok(())
    }

    pub async fn all_medias(&self) -> Result<Vec<media::Model>> {
        media::Entity::find()
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn all_pending_medias(&self) -> Result<Vec<media::Model>> {
        media::Entity::find()
            .filter(media::Column::State.eq(MediaState::Pending.to_string()))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }
}
