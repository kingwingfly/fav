use anyhow::{Context as _, Result};
use fav::migration::OnConflict;
use sea_orm::{
    ActiveValue::{Set, Unchanged},
    ConnectionTrait, DatabaseBackend, EntityTrait as _, IntoActiveModel as _, Statement,
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
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn all_medias(&self) -> Result<Vec<media::Model>> {
        media::Entity::find()
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn all_active_pending_medias(&self) -> Result<Vec<media::Model>> {
        media::Entity::find()
            .from_raw_sql(Statement::from_string(
                DatabaseBackend::Sqlite,
                r#"
SELECT DISTINCT m.*
FROM media m
WHERE
m.state = 'Pending'
AND (
    EXISTS (
        SELECT 1
        FROM media_up mu
        JOIN up u ON mu.up_id = u.up_id
        WHERE mu.id = m.id AND u.state = 'Active'
    )
    OR EXISTS (
        SELECT 1
        FROM media_set ms
        JOIN "set" s ON ms.set_id = s.set_id
        WHERE ms.id = m.id AND s.state = 'Active'
    )
);
"#,
            ))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    /// Cleanup the medias whose up and set both are inactive/null
    pub async fn prune_medias(&self) -> Result<()> {
        self.db
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                r#"
DELETE FROM media
WHERE id IN (
    SELECT m.id
    FROM media m
    WHERE NOT EXISTS (
        SELECT 1 FROM media_up mu
        JOIN up u ON mu.up_id = u.up_id
        WHERE mu.id = m.id AND u.state != 'Inactive'
    )
    AND NOT EXISTS (
        SELECT 1 FROM media_set ms
        JOIN "set" s ON ms.set_id = s.set_id
        WHERE ms.id = m.id AND s.state != 'Inactive'
    )
);
"#,
            ))
            .await?;
        Ok(())
    }
}
