use anyhow::Result;
use sea_orm::{
    ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _,
    TransactionTrait as _,
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

    pub async fn get_up_ids_of_media(&self, media_id: i64) -> Result<Vec<i64>> {
        media_up::Entity::find()
            .filter(media_up::Column::Id.eq(media_id))
            .all(&self.conn)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.up_id).collect())
    }
}
