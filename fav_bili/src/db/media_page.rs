use std::collections::HashSet;

use anyhow::{Result, anyhow};
use sea_orm::{
    ColumnTrait as _, Condition, EntityTrait as _, IntoActiveModel as _, QueryFilter as _,
    QueryOrder as _, TransactionTrait as _,
    sea_query::{Expr, OnConflict},
};

use super::{CHUNK_SIZE, Db, now_unix};
use crate::{entity::media_page, state::MediaPageState};

impl Db {
    pub async fn upsert_media_page(&self, page: media_page::Model) -> Result<()> {
        self.upsert_media_pages([page]).await
    }

    pub async fn upsert_media_pages(
        &self,
        pages: impl IntoIterator<Item = media_page::Model>,
    ) -> Result<()> {
        let items: Vec<media_page::Model> = pages.into_iter().collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            media_page::Entity::insert_many(chunk.iter().cloned().map(|p| p.into_active_model()))
                .on_conflict(
                    OnConflict::columns([media_page::Column::MediaId, media_page::Column::Cid])
                        .update_columns([
                            media_page::Column::Page,
                            media_page::Column::Part,
                            media_page::Column::State,
                            media_page::Column::LocalPath,
                            media_page::Column::DownloadedAt,
                            media_page::Column::FileSize,
                            media_page::Column::FileInode,
                        ])
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn mark_page_completed(
        &self,
        media_id: i64,
        cid: i64,
        page: i32,
        part: String,
        local_path: String,
        file_size: Option<i64>,
        file_inode: Option<i64>,
    ) -> Result<()> {
        self.upsert_media_page(media_page::Model {
            media_id,
            cid,
            page,
            part,
            state: MediaPageState::Completed.to_string(),
            local_path: Some(local_path),
            downloaded_at: Some(now_unix()),
            file_size,
            file_inode,
        })
        .await
    }

    pub async fn mark_page_failed(
        &self,
        media_id: i64,
        cid: i64,
        page: i32,
        part: String,
    ) -> Result<()> {
        self.upsert_media_page(media_page::Model {
            media_id,
            cid,
            page,
            part,
            state: MediaPageState::Failed.to_string(),
            local_path: None,
            downloaded_at: None,
            file_size: None,
            file_inode: None,
        })
        .await
    }

    pub async fn update_page_local_path(
        &self,
        media_id: i64,
        cid: i64,
        local_path: String,
        file_size: Option<i64>,
        file_inode: Option<i64>,
    ) -> Result<()> {
        let res = media_page::Entity::update_many()
            .col_expr(media_page::Column::LocalPath, Expr::value(Some(local_path)))
            .col_expr(media_page::Column::FileSize, Expr::value(file_size))
            .col_expr(media_page::Column::FileInode, Expr::value(file_inode))
            .filter(
                Condition::all()
                    .add(media_page::Column::MediaId.eq(media_id))
                    .add(media_page::Column::Cid.eq(cid)),
            )
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!(
                "Unknown media_page<media_id={media_id}, cid={cid}>"
            ));
        }
        Ok(())
    }

    pub async fn pages_for_media(&self, media_id: i64) -> Result<Vec<media_page::Model>> {
        media_page::Entity::find()
            .filter(media_page::Column::MediaId.eq(media_id))
            .order_by_asc(media_page::Column::Page)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn pages_for_media_ids(&self, media_ids: &[i64]) -> Result<Vec<media_page::Model>> {
        if media_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        for chunk in media_ids.chunks(CHUNK_SIZE) {
            let rows = media_page::Entity::find()
                .filter(media_page::Column::MediaId.is_in(chunk.iter().copied()))
                .all(&self.conn)
                .await?;
            results.extend(rows);
        }
        results.sort_by_key(|p| (p.media_id, p.page, p.cid));
        Ok(results)
    }

    pub async fn all_completed_pages(&self) -> Result<Vec<media_page::Model>> {
        media_page::Entity::find()
            .filter(media_page::Column::State.eq(MediaPageState::Completed))
            .order_by_asc(media_page::Column::MediaId)
            .order_by_asc(media_page::Column::Page)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn reset_pages_for_media(&self, media_id: i64) -> Result<u64> {
        let res = media_page::Entity::update_many()
            .col_expr(
                media_page::Column::State,
                Expr::value(MediaPageState::Pending.as_str()),
            )
            .col_expr(
                media_page::Column::LocalPath,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                media_page::Column::DownloadedAt,
                Expr::value(Option::<i64>::None),
            )
            .col_expr(
                media_page::Column::FileSize,
                Expr::value(Option::<i64>::None),
            )
            .col_expr(
                media_page::Column::FileInode,
                Expr::value(Option::<i64>::None),
            )
            .filter(media_page::Column::MediaId.eq(media_id))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }

    /// Surgically reset only the specified `(media_id, cid)` pages back to `Pending`,
    /// leaving all other pages of the same media untouched.
    pub async fn reset_specific_pages(&self, pairs: &[(i64, i64)]) -> Result<u64> {
        if pairs.is_empty() {
            return Ok(0);
        }
        let unique: HashSet<(i64, i64)> = pairs.iter().copied().collect();
        let txn = self.conn.begin().await?;
        let mut total = 0u64;
        for (media_id, cid) in unique {
            let res = media_page::Entity::update_many()
                .col_expr(
                    media_page::Column::State,
                    Expr::value(MediaPageState::Pending.as_str()),
                )
                .col_expr(
                    media_page::Column::LocalPath,
                    Expr::value(Option::<String>::None),
                )
                .col_expr(
                    media_page::Column::DownloadedAt,
                    Expr::value(Option::<i64>::None),
                )
                .col_expr(
                    media_page::Column::FileSize,
                    Expr::value(Option::<i64>::None),
                )
                .col_expr(
                    media_page::Column::FileInode,
                    Expr::value(Option::<i64>::None),
                )
                .filter(
                    Condition::all()
                        .add(media_page::Column::MediaId.eq(media_id))
                        .add(media_page::Column::Cid.eq(cid)),
                )
                .exec(&txn)
                .await?;
            total += res.rows_affected;
        }
        txn.commit().await?;
        Ok(total)
    }

    pub async fn delete_pages_for_media(&self, media_id: i64) -> Result<u64> {
        let res = media_page::Entity::delete_many()
            .filter(media_page::Column::MediaId.eq(media_id))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }
}
