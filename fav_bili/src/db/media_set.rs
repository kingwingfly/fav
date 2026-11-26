use std::collections::HashSet;

use anyhow::Result;
use sea_orm::{
    ColumnTrait as _, Condition, EntityTrait as _, IntoActiveModel as _, QueryFilter as _,
    TransactionTrait as _,
};

use super::{CHUNK_SIZE, Db};
use crate::entity::media_set;

impl Db {
    pub async fn upsert_media_sets(
        &self,
        media_sets: impl IntoIterator<Item = media_set::Model>,
    ) -> Result<()> {
        let items: Vec<media_set::Model> = media_sets.into_iter().collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            media_set::Entity::insert_many(chunk.iter().cloned().map(|m| m.into_active_model()))
                .on_conflict_do_nothing()
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_media_ids_of_set(&self, set_id: i64) -> Result<Vec<i64>> {
        media_set::Entity::find()
            .filter(media_set::Column::SetId.eq(set_id))
            .all(&self.conn)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.id).collect())
    }

    /// Remove `media_set` links for `set_id` whose media `id` is no longer in `keep_media_ids`.
    /// Call only when all pages of the favorite set were fetched cleanly.
    pub async fn unlink_stale_media_sets(
        &self,
        set_id: i64,
        keep_media_ids: &[i64],
    ) -> Result<u64> {
        let keep: HashSet<i64> = keep_media_ids.iter().copied().collect();
        let txn = self.conn.begin().await?;
        let existing: Vec<i64> = media_set::Entity::find()
            .filter(media_set::Column::SetId.eq(set_id))
            .all(&txn)
            .await?
            .into_iter()
            .map(|m| m.id)
            .collect();
        let stale: Vec<i64> = existing
            .into_iter()
            .filter(|id| !keep.contains(id))
            .collect();
        let mut removed = 0u64;
        for chunk in stale.chunks(CHUNK_SIZE) {
            let res = media_set::Entity::delete_many()
                .filter(
                    Condition::all()
                        .add(media_set::Column::SetId.eq(set_id))
                        .add(media_set::Column::Id.is_in(chunk.iter().copied())),
                )
                .exec(&txn)
                .await?;
            removed += res.rows_affected;
        }
        txn.commit().await?;
        Ok(removed)
    }
}
