use std::collections::HashSet;

use anyhow::{Context as _, Result, anyhow};
use sea_orm::{
    ColumnTrait as _, Condition, ConnectionTrait as _, DatabaseBackend, EntityTrait as _,
    IntoActiveModel as _, QueryFilter as _, QueryOrder as _, QuerySelect as _, Statement,
    TransactionTrait as _,
    sea_query::{Expr, OnConflict, Query},
};

use super::{CHUNK_SIZE, Db};
use crate::{
    entity::{account, up, up_account},
    state::{AccountState, UpState},
};

#[derive(Debug, Clone, Default)]
pub struct UpFilter {
    pub states: Vec<UpState>,
    pub active_only: bool,
    pub account_id: Option<i64>,
    pub search: Option<String>,
    pub limit: Option<u64>,
}

impl Db {
    pub async fn upsert_ups(&self, ups: impl IntoIterator<Item = up::Model>) -> Result<()> {
        let items: Vec<up::Model> = ups.into_iter().collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            up::Entity::insert_many(chunk.iter().cloned().map(|s| s.into_active_model()))
                .on_conflict(
                    OnConflict::column(up::Column::UpId)
                        .update_columns([up::Column::Name])
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Atomically upsert followed UPs and `up_account` links.
    /// Only unlinks stale `up_account` entries when `unlink_stale` is `true` (all pages succeeded),
    /// including when `ups` is empty (account now follows 0 UPs).
    pub async fn sync_account_ups(
        &self,
        account_id: i64,
        ups: Vec<up::Model>,
        unlink_stale: bool,
    ) -> Result<Vec<i64>> {
        let txn = self.conn.begin().await?;
        if !ups.is_empty() {
            for chunk in ups.chunks(CHUNK_SIZE) {
                up::Entity::insert_many(chunk.iter().cloned().map(|u| u.into_active_model()))
                    .on_conflict(
                        OnConflict::column(up::Column::UpId)
                            .update_columns([up::Column::Name])
                            .to_owned(),
                    )
                    .exec_without_returning(&txn)
                    .await?;
            }
            let links: Vec<up_account::Model> = ups
                .iter()
                .map(|u| up_account::Model {
                    up_id: u.up_id,
                    account_id,
                })
                .collect();
            for chunk in links.chunks(CHUNK_SIZE) {
                up_account::Entity::insert_many(
                    chunk.iter().cloned().map(|m| m.into_active_model()),
                )
                .on_conflict_do_nothing()
                .exec_without_returning(&txn)
                .await?;
            }
        }

        let mut stale = Vec::new();
        if unlink_stale {
            let current_ids: HashSet<i64> = ups.iter().map(|u| u.up_id).collect();
            let existing_ids: Vec<i64> = up_account::Entity::find()
                .filter(up_account::Column::AccountId.eq(account_id))
                .all(&txn)
                .await?
                .into_iter()
                .map(|m| m.up_id)
                .collect();
            stale = existing_ids
                .into_iter()
                .filter(|id| !current_ids.contains(id))
                .collect();
            for chunk in stale.chunks(CHUNK_SIZE) {
                up_account::Entity::delete_many()
                    .filter(
                        Condition::all()
                            .add(up_account::Column::AccountId.eq(account_id))
                            .add(up_account::Column::UpId.is_in(chunk.iter().copied())),
                    )
                    .exec(&txn)
                    .await?;
            }
        }
        txn.commit().await?;
        Ok(stale)
    }

    pub async fn set_up_sync_progress(
        &self,
        up_id: i64,
        sync_complete: bool,
        resume_page: Option<i32>,
    ) -> Result<()> {
        let res = up::Entity::update_many()
            .col_expr(up::Column::SyncComplete, Expr::value(sync_complete))
            .col_expr(up::Column::ResumePage, Expr::value(resume_page))
            .filter(up::Column::UpId.eq(up_id))
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown up<{up_id}>"));
        }
        Ok(())
    }

    pub async fn is_up_followed_by_active_account(&self, up_id: i64) -> Result<bool> {
        let active_accounts = Query::select()
            .column(account::Column::AccountId)
            .from(account::Entity)
            .and_where(Expr::col(account::Column::State).eq(AccountState::Active.as_str()))
            .to_owned();
        let found = up_account::Entity::find()
            .filter(
                Condition::all()
                    .add(up_account::Column::UpId.eq(up_id))
                    .add(up_account::Column::AccountId.in_subquery(active_accounts)),
            )
            .one(&self.conn)
            .await?;
        Ok(found.is_some())
    }

    pub async fn active_account_up_ids(&self) -> Result<HashSet<i64>> {
        let active_accounts = Query::select()
            .column(account::Column::AccountId)
            .from(account::Entity)
            .and_where(Expr::col(account::Column::State).eq(AccountState::Active.as_str()))
            .to_owned();
        let rows = up_account::Entity::find()
            .filter(up_account::Column::AccountId.in_subquery(active_accounts))
            .all(&self.conn)
            .await?;
        Ok(rows.into_iter().map(|r| r.up_id).collect())
    }

    pub async fn get_up(&self, up_id: i64) -> Result<up::Model> {
        up::Entity::find_by_id(up_id)
            .one(&self.conn)
            .await?
            .context(format!("Unknown up<{up_id}>"))
    }

    pub async fn all_ups(&self) -> Result<Vec<up::Model>> {
        up::Entity::find()
            .order_by_asc(up::Column::UpId)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn query_ups(&self, filter: &UpFilter) -> Result<Vec<up::Model>> {
        let mut query = up::Entity::find().order_by_asc(up::Column::UpId);
        if filter.active_only {
            query = query.filter(up::Column::State.eq(UpState::Active));
        } else if !filter.states.is_empty() {
            let states: Vec<String> = filter.states.iter().map(|s| s.to_string()).collect();
            query = query.filter(up::Column::State.is_in(states));
        }
        if let Some(account_id) = filter.account_id {
            let sub = Query::select()
                .column(up_account::Column::UpId)
                .from(up_account::Entity)
                .and_where(Expr::col(up_account::Column::AccountId).eq(account_id))
                .to_owned();
            query = query.filter(up::Column::UpId.in_subquery(sub));
        }
        if let Some(search) = &filter.search
            && !search.trim().is_empty()
        {
            let pattern = format!("%{}%", search.trim().to_ascii_lowercase());
            query = query.filter(Expr::cust_with_values("LOWER(name) LIKE ?", [pattern]));
        }
        if let Some(limit) = filter.limit {
            query = query.limit(limit);
        }
        query.all(&self.conn).await.map_err(Into::into)
    }

    pub async fn set_up_state(&self, up_id: i64, state: UpState) -> Result<()> {
        let res = up::Entity::update_many()
            .col_expr(up::Column::State, Expr::value(state.as_str()))
            .filter(up::Column::UpId.eq(up_id))
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown up<{up_id}>"));
        }
        Ok(())
    }

    pub async fn activate_up(&self, up_id: i64) -> Result<()> {
        self.set_up_state(up_id, UpState::Active).await
    }

    pub async fn activate_ups(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for &id in ids {
            let res = up::Entity::update_many()
                .col_expr(up::Column::State, Expr::value(UpState::Active.as_str()))
                .filter(up::Column::UpId.eq(id))
                .exec(&txn)
                .await?;
            if res.rows_affected == 0 {
                return Err(anyhow!("Unknown up<{id}>"));
            }
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn activate_all_ups(&self) -> Result<u64> {
        let res = up::Entity::update_many()
            .col_expr(up::Column::State, Expr::value(UpState::Active.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn deactivate_up(&self, up_id: i64) -> Result<()> {
        self.set_up_state(up_id, UpState::Inactive).await
    }

    pub async fn deactivate_ups(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for &id in ids {
            let res = up::Entity::update_many()
                .col_expr(up::Column::State, Expr::value(UpState::Inactive.as_str()))
                .filter(up::Column::UpId.eq(id))
                .exec(&txn)
                .await?;
            if res.rows_affected == 0 {
                return Err(anyhow!("Unknown up<{id}>"));
            }
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn deactivate_all_ups(&self) -> Result<u64> {
        let res = up::Entity::update_many()
            .col_expr(up::Column::State, Expr::value(UpState::Inactive.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }

    /// Cleanup UPs followed by no account and linked to no remaining media.
    pub async fn prune_ups(&self) -> Result<u64> {
        let res = self
            .conn
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                PRUNE_UPS_SQL,
            ))
            .await?;
        Ok(res.rows_affected())
    }
}

pub(crate) const PRUNE_UPS_SQL: &str = r#"
DELETE FROM up
WHERE up_id IN (
    SELECT u.up_id
    FROM up u
    WHERE NOT EXISTS (
        SELECT 1 FROM up_account ua
        JOIN account a ON ua.account_id = a.account_id
        WHERE ua.up_id = u.up_id
    )
    AND NOT EXISTS (
        SELECT 1 FROM media_up mu
        JOIN media m ON mu.id = m.id
        WHERE mu.up_id = u.up_id
    )
);
"#;
