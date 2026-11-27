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
    entity::{account, set, set_account},
    state::{AccountState, SetKind, SetState},
};

#[derive(Debug, Clone, Default)]
pub struct SetFilter {
    pub states: Vec<SetState>,
    pub active_only: bool,
    pub account_id: Option<i64>,
    pub kind: Option<SetKind>,
    pub search: Option<String>,
    pub limit: Option<u64>,
}

impl Db {
    pub async fn upsert_sets(&self, sets: impl IntoIterator<Item = set::Model>) -> Result<()> {
        let items: Vec<set::Model> = sets.into_iter().collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            set::Entity::insert_many(chunk.iter().cloned().map(|s| s.into_active_model()))
                .on_conflict(
                    OnConflict::column(set::Column::SetId)
                        .update_columns([
                            set::Column::Name,
                            set::Column::Count,
                            set::Column::Kind,
                            set::Column::OwnerMid,
                        ])
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Atomically upsert an account's sets, link `set_account`, and optionally remove stale
    /// `set_account` rows when `unlink_stale` is `true` (all catalog pages succeeded).
    /// Preserves existing `state`, `sync_complete`, and `resume_page`.
    /// Returns the list of unlinked `set_id`s.
    pub async fn sync_account_sets(
        &self,
        account_id: i64,
        sets: Vec<set::Model>,
        unlink_stale: bool,
    ) -> Result<Vec<i64>> {
        let txn = self.conn.begin().await?;
        if !sets.is_empty() {
            for chunk in sets.chunks(CHUNK_SIZE) {
                set::Entity::insert_many(chunk.iter().cloned().map(|s| s.into_active_model()))
                    .on_conflict(
                        OnConflict::column(set::Column::SetId)
                            .update_columns([
                                set::Column::Name,
                                set::Column::Count,
                                set::Column::Kind,
                                set::Column::OwnerMid,
                            ])
                            .to_owned(),
                    )
                    .exec_without_returning(&txn)
                    .await?;
            }
            let links: Vec<set_account::Model> = sets
                .iter()
                .map(|s| set_account::Model {
                    set_id: s.set_id,
                    account_id,
                })
                .collect();
            for chunk in links.chunks(CHUNK_SIZE) {
                set_account::Entity::insert_many(
                    chunk.iter().cloned().map(|m| m.into_active_model()),
                )
                .on_conflict_do_nothing()
                .exec_without_returning(&txn)
                .await?;
            }
        }

        let mut stale = Vec::new();
        if unlink_stale {
            let current_ids: HashSet<i64> = sets.iter().map(|s| s.set_id).collect();
            let existing_ids: Vec<i64> = set_account::Entity::find()
                .filter(set_account::Column::AccountId.eq(account_id))
                .all(&txn)
                .await?
                .into_iter()
                .map(|m| m.set_id)
                .collect();
            stale = existing_ids
                .into_iter()
                .filter(|id| !current_ids.contains(id))
                .collect();
            for chunk in stale.chunks(CHUNK_SIZE) {
                set_account::Entity::delete_many()
                    .filter(
                        Condition::all()
                            .add(set_account::Column::AccountId.eq(account_id))
                            .add(set_account::Column::SetId.is_in(chunk.iter().copied())),
                    )
                    .exec(&txn)
                    .await?;
            }
        }
        txn.commit().await?;
        Ok(stale)
    }

    pub async fn set_set_sync_progress(
        &self,
        set_id: i64,
        sync_complete: bool,
        resume_page: Option<i32>,
    ) -> Result<()> {
        let res = set::Entity::update_many()
            .col_expr(set::Column::SyncComplete, Expr::value(sync_complete))
            .col_expr(set::Column::ResumePage, Expr::value(resume_page))
            .filter(set::Column::SetId.eq(set_id))
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown set<{set_id}>"));
        }
        Ok(())
    }

    pub async fn is_set_owned_by_active_account(&self, set_id: i64) -> Result<bool> {
        let active_accounts = Query::select()
            .column(account::Column::AccountId)
            .from(account::Entity)
            .and_where(Expr::col(account::Column::State).eq(AccountState::Active.as_str()))
            .to_owned();
        let found = set_account::Entity::find()
            .filter(
                Condition::all()
                    .add(set_account::Column::SetId.eq(set_id))
                    .add(set_account::Column::AccountId.in_subquery(active_accounts)),
            )
            .one(&self.conn)
            .await?;
        Ok(found.is_some())
    }

    pub async fn active_account_set_ids(&self) -> Result<HashSet<i64>> {
        let active_accounts = Query::select()
            .column(account::Column::AccountId)
            .from(account::Entity)
            .and_where(Expr::col(account::Column::State).eq(AccountState::Active.as_str()))
            .to_owned();
        let rows = set_account::Entity::find()
            .filter(set_account::Column::AccountId.in_subquery(active_accounts))
            .all(&self.conn)
            .await?;
        Ok(rows.into_iter().map(|r| r.set_id).collect())
    }

    pub async fn get_set(&self, set_id: i64) -> Result<set::Model> {
        set::Entity::find_by_id(set_id)
            .one(&self.conn)
            .await?
            .context(format!("Unknown set<{set_id}>"))
    }

    pub async fn all_sets(&self) -> Result<Vec<set::Model>> {
        set::Entity::find()
            .order_by_asc(set::Column::SetId)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn query_sets(&self, filter: &SetFilter) -> Result<Vec<set::Model>> {
        let mut query = set::Entity::find().order_by_asc(set::Column::SetId);
        if filter.active_only {
            query = query.filter(set::Column::State.eq(SetState::Active));
        } else if !filter.states.is_empty() {
            let states: Vec<String> = filter.states.iter().map(|s| s.to_string()).collect();
            query = query.filter(set::Column::State.is_in(states));
        }
        if let Some(account_id) = filter.account_id {
            let sub = Query::select()
                .column(set_account::Column::SetId)
                .from(set_account::Entity)
                .and_where(Expr::col(set_account::Column::AccountId).eq(account_id))
                .to_owned();
            query = query.filter(set::Column::SetId.in_subquery(sub));
        }
        if let Some(kind) = filter.kind {
            query = query.filter(set::Column::Kind.eq(kind));
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

    pub async fn set_set_state(&self, set_id: i64, state: SetState) -> Result<()> {
        let res = set::Entity::update_many()
            .col_expr(set::Column::State, Expr::value(state.as_str()))
            .filter(set::Column::SetId.eq(set_id))
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown set<{set_id}>"));
        }
        Ok(())
    }

    pub async fn activate_set(&self, set_id: i64) -> Result<()> {
        self.set_set_state(set_id, SetState::Active).await
    }

    pub async fn activate_sets(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for &id in ids {
            let res = set::Entity::update_many()
                .col_expr(set::Column::State, Expr::value(SetState::Active.as_str()))
                .filter(set::Column::SetId.eq(id))
                .exec(&txn)
                .await?;
            if res.rows_affected == 0 {
                return Err(anyhow!("Unknown set<{id}>"));
            }
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn activate_all_sets(&self) -> Result<u64> {
        let res = set::Entity::update_many()
            .col_expr(set::Column::State, Expr::value(SetState::Active.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn deactivate_set(&self, set_id: i64) -> Result<()> {
        self.set_set_state(set_id, SetState::Inactive).await
    }

    pub async fn deactivate_sets(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for &id in ids {
            let res = set::Entity::update_many()
                .col_expr(set::Column::State, Expr::value(SetState::Inactive.as_str()))
                .filter(set::Column::SetId.eq(id))
                .exec(&txn)
                .await?;
            if res.rows_affected == 0 {
                return Err(anyhow!("Unknown set<{id}>"));
            }
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn deactivate_all_sets(&self) -> Result<u64> {
        let res = set::Entity::update_many()
            .col_expr(set::Column::State, Expr::value(SetState::Inactive.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }

    /// Cleanup sets belonging to no account.
    pub async fn prune_sets(&self) -> Result<u64> {
        let res = self
            .conn
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                PRUNE_SETS_SQL,
            ))
            .await?;
        Ok(res.rows_affected())
    }
}

pub(crate) const PRUNE_SETS_SQL: &str = r#"
DELETE FROM "set"
WHERE set_id IN (
    SELECT s.set_id
    FROM "set" s
    WHERE NOT EXISTS (
        SELECT 1 FROM set_account sa
        JOIN account a ON sa.account_id = a.account_id
        WHERE sa.set_id = s.set_id
    )
);
"#;
