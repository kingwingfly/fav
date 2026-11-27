use anyhow::Result;
use sea_orm::{
    ColumnTrait as _, Condition, EntityTrait as _, IntoActiveModel as _, QueryFilter as _,
    TransactionTrait as _, sea_query::OnConflict,
};

use super::{CHUNK_SIZE, Db};
use crate::entity::media_up;

impl Db {
    pub async fn upsert_media_ups(
        &self,
        media_ups: impl IntoIterator<Item = media_up::Model>,
    ) -> Result<()> {
        let items: Vec<media_up::Model> = media_ups.into_iter().collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            media_up::Entity::insert_many(chunk.iter().cloned().map(|m| m.into_active_model()))
                .on_conflict_do_nothing()
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Upsert `media_up` links observed directly in a UP's published channel feed.
    /// Always persists `from_up_feed = true`, upgrading any existing `(id, up_id)` row previously
    /// discovered via favorite sets or `/view` backfill from `false` -> `true`.
    pub async fn upsert_up_feed_media_ups(
        &self,
        media_ups: impl IntoIterator<Item = media_up::Model>,
    ) -> Result<()> {
        let items: Vec<media_up::Model> = media_ups
            .into_iter()
            .map(|mut m| {
                m.from_up_feed = true;
                m
            })
            .collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            media_up::Entity::insert_many(chunk.iter().cloned().map(|m| m.into_active_model()))
                .on_conflict(
                    OnConflict::columns([media_up::Column::Id, media_up::Column::UpId])
                        .update_column(media_up::Column::FromUpFeed)
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_up_feed_media_ids(&self, up_id: i64) -> Result<Vec<i64>> {
        media_up::Entity::find()
            .filter(
                Condition::all()
                    .add(media_up::Column::UpId.eq(up_id))
                    .add(media_up::Column::FromUpFeed.eq(true)),
            )
            .all(&self.conn)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.id).collect())
    }

    pub async fn get_up_ids_of_media(&self, media_id: i64) -> Result<Vec<i64>> {
        media_up::Entity::find()
            .filter(media_up::Column::Id.eq(media_id))
            .all(&self.conn)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.up_id).collect())
    }
}
