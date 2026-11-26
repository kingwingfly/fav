use anyhow::Result;
use sea_orm::{
    ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _, QueryOrder as _,
    TransactionTrait as _,
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
                        ])
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn mark_page_completed(
        &self,
        media_id: i64,
        cid: i64,
        page: i32,
        part: String,
        local_path: String,
    ) -> Result<()> {
        self.upsert_media_page(media_page::Model {
            media_id,
            cid,
            page,
            part,
            state: MediaPageState::Completed.to_string(),
            local_path: Some(local_path),
            downloaded_at: Some(now_unix()),
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
        })
        .await
    }

    pub async fn pages_for_media(&self, media_id: i64) -> Result<Vec<media_page::Model>> {
        media_page::Entity::find()
            .filter(media_page::Column::MediaId.eq(media_id))
            .order_by_asc(media_page::Column::Page)
            .all(&self.conn)
            .await
            .map_err(Into::into)
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
            .filter(media_page::Column::MediaId.eq(media_id))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn delete_pages_for_media(&self, media_id: i64) -> Result<u64> {
        let res = media_page::Entity::delete_many()
            .filter(media_page::Column::MediaId.eq(media_id))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }
}
