use std::collections::HashSet;

use anyhow::{Context as _, Result, anyhow};
use sea_orm::{
    ColumnTrait as _, Condition, ConnectionTrait as _, DatabaseBackend, EntityTrait as _,
    IntoActiveModel as _, PaginatorTrait as _, QueryFilter as _, QueryOrder as _, QuerySelect as _,
    Statement, TransactionTrait as _,
    sea_query::{Expr, OnConflict, Query},
};
use serde::Serialize;

use super::{CHUNK_SIZE, Db, now_unix, set::PRUNE_SETS_SQL, up::PRUNE_UPS_SQL};
use crate::{
    entity::{account, media, media_set, media_up, set, set_account, up, up_account},
    state::{AccountState, MediaState, SetState, UpState},
};

const MAX_ERROR_LEN: usize = 512;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MediaSort {
    #[default]
    Id,
    Title,
    State,
    UpdatedAt,
}

#[derive(Debug, Clone, Default)]
pub struct MediaFilter {
    pub states: Vec<MediaState>,
    pub set_id: Option<i64>,
    pub up_id: Option<i64>,
    pub account_id: Option<i64>,
    pub active_only: bool,
    pub media_type: Option<String>,
    pub search: Option<String>,
    pub sort: MediaSort,
    pub desc: bool,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct PullFilter {
    pub ids: Vec<i64>,
    pub set_ids: Vec<i64>,
    pub up_ids: Vec<i64>,
    pub account_id: Option<i64>,
    pub retry_failed: bool,
    pub force: bool,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct MediaResetFilter {
    pub ids: Vec<i64>,
    pub failed: bool,
    pub expired: bool,
    pub permission_denied: bool,
    pub skipped: bool,
    pub completed: bool,
    pub all: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PruneReport {
    pub sets_pruned: u64,
    pub medias_pruned: u64,
    pub ups_pruned: u64,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AccountCounts {
    pub total: u64,
    pub active: u64,
    pub inactive: u64,
    pub expired: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SetCounts {
    pub total: u64,
    pub active: u64,
    pub inactive: u64,
    pub unreachable: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UpCounts {
    pub total: u64,
    pub active: u64,
    pub inactive: u64,
    pub deactivated: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MediaCounts {
    pub total: u64,
    pub pending: u64,
    pub downloading: u64,
    pub completed: u64,
    pub failed: u64,
    pub expired: u64,
    pub permission_denied: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopUpStat {
    pub up_id: i64,
    pub name: String,
    pub media_count: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StatusSummary {
    pub accounts: AccountCounts,
    pub sets: SetCounts,
    pub ups: UpCounts,
    pub medias: MediaCounts,
    pub top_ups: Vec<TopUpStat>,
}

fn active_media_condition_for_accounts(account_ids: Option<&[i64]>) -> Condition {
    let mut active_up_account = Query::select();
    active_up_account
        .expr(Expr::val(1))
        .from(up_account::Entity)
        .inner_join(
            account::Entity,
            Expr::col((up_account::Entity, up_account::Column::AccountId))
                .equals((account::Entity, account::Column::AccountId)),
        )
        .and_where(
            Expr::col((up_account::Entity, up_account::Column::UpId))
                .equals((up::Entity, up::Column::UpId)),
        )
        .and_where(
            Expr::col((account::Entity, account::Column::State)).eq(AccountState::Active.as_str()),
        );
    if let Some(ids) = account_ids {
        active_up_account.and_where(
            Expr::col((account::Entity, account::Column::AccountId)).is_in(ids.iter().copied()),
        );
    }

    let active_up_sub = Query::select()
        .expr(Expr::val(1))
        .from(media_up::Entity)
        .inner_join(
            up::Entity,
            Expr::col((media_up::Entity, media_up::Column::UpId))
                .equals((up::Entity, up::Column::UpId)),
        )
        .and_where(
            Expr::col((media_up::Entity, media_up::Column::Id))
                .equals((media::Entity, media::Column::Id)),
        )
        .and_where(Expr::col((up::Entity, up::Column::State)).eq(UpState::Active.as_str()))
        .and_where(Expr::exists(active_up_account.to_owned()))
        .to_owned();

    let mut active_set_account = Query::select();
    active_set_account
        .expr(Expr::val(1))
        .from(set_account::Entity)
        .inner_join(
            account::Entity,
            Expr::col((set_account::Entity, set_account::Column::AccountId))
                .equals((account::Entity, account::Column::AccountId)),
        )
        .and_where(
            Expr::col((set_account::Entity, set_account::Column::SetId))
                .equals((set::Entity, set::Column::SetId)),
        )
        .and_where(
            Expr::col((account::Entity, account::Column::State)).eq(AccountState::Active.as_str()),
        );
    if let Some(ids) = account_ids {
        active_set_account.and_where(
            Expr::col((account::Entity, account::Column::AccountId)).is_in(ids.iter().copied()),
        );
    }

    let active_set_sub = Query::select()
        .expr(Expr::val(1))
        .from(media_set::Entity)
        .inner_join(
            set::Entity,
            Expr::col((media_set::Entity, media_set::Column::SetId))
                .equals((set::Entity, set::Column::SetId)),
        )
        .and_where(
            Expr::col((media_set::Entity, media_set::Column::Id))
                .equals((media::Entity, media::Column::Id)),
        )
        .and_where(Expr::col((set::Entity, set::Column::State)).eq(SetState::Active.as_str()))
        .and_where(Expr::exists(active_set_account.to_owned()))
        .to_owned();

    Condition::any()
        .add(Expr::exists(active_up_sub))
        .add(Expr::exists(active_set_sub))
}

fn active_media_condition() -> Condition {
    active_media_condition_for_accounts(None)
}

fn active_media_condition_for_account(account_id: i64) -> Condition {
    active_media_condition_for_accounts(Some(&[account_id]))
}

fn account_media_condition(account_id: i64) -> Condition {
    let via_set = Query::select()
        .expr(Expr::val(1))
        .from(media_set::Entity)
        .inner_join(
            set_account::Entity,
            Expr::col((media_set::Entity, media_set::Column::SetId))
                .equals((set_account::Entity, set_account::Column::SetId)),
        )
        .and_where(
            Expr::col((media_set::Entity, media_set::Column::Id))
                .equals((media::Entity, media::Column::Id)),
        )
        .and_where(Expr::col((set_account::Entity, set_account::Column::AccountId)).eq(account_id))
        .to_owned();

    let via_up = Query::select()
        .expr(Expr::val(1))
        .from(media_up::Entity)
        .inner_join(
            up_account::Entity,
            Expr::col((media_up::Entity, media_up::Column::UpId))
                .equals((up_account::Entity, up_account::Column::UpId)),
        )
        .and_where(
            Expr::col((media_up::Entity, media_up::Column::Id))
                .equals((media::Entity, media::Column::Id)),
        )
        .and_where(Expr::col((up_account::Entity, up_account::Column::AccountId)).eq(account_id))
        .to_owned();

    Condition::any()
        .add(Expr::exists(via_set))
        .add(Expr::exists(via_up))
}

impl Db {
    /// Bulk upsert media rows in chunks inside a transaction.
    /// Conflicts on primary key `media.id` (never mutating PK `id`) and preserves existing `state`,
    /// `retry_count`, and `last_error`.
    pub async fn upsert_medias(
        &self,
        medias: impl IntoIterator<Item = media::Model>,
    ) -> Result<()> {
        let now = now_unix();
        let items: Vec<media::Model> = medias
            .into_iter()
            .map(|mut m| {
                if m.bv_id.trim().is_empty() {
                    m.bv_id = format!("av{}", m.id);
                }
                if m.updated_at.is_none() {
                    m.updated_at = Some(now);
                }
                m
            })
            .collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            media::Entity::insert_many(chunk.iter().cloned().map(|m| m.into_active_model()))
                .on_conflict(
                    OnConflict::column(media::Column::Id)
                        .update_columns([
                            media::Column::BvId,
                            media::Column::Title,
                            media::Column::Type,
                            media::Column::UpdatedAt,
                        ])
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_media(&self, id: i64) -> Result<media::Model> {
        media::Entity::find_by_id(id)
            .one(&self.conn)
            .await?
            .context(format!("Unknown media<{id}>"))
    }

    pub async fn get_media_by_bvid(&self, bvid: &str) -> Result<Option<media::Model>> {
        media::Entity::find()
            .filter(media::Column::BvId.eq(bvid))
            .one(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn set_media_state(&self, id: i64, state: MediaState) -> Result<()> {
        let mut query = media::Entity::update_many()
            .col_expr(media::Column::State, Expr::value(state.as_str()))
            .col_expr(media::Column::UpdatedAt, Expr::value(Some(now_unix())))
            .filter(media::Column::Id.eq(id));
        if matches!(
            state,
            MediaState::Completed | MediaState::Pending | MediaState::Skipped
        ) {
            query = query.col_expr(
                media::Column::LastError,
                Expr::value(Option::<String>::None),
            );
        }
        let res = query.exec(&self.conn).await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown media<{id}>"));
        }
        Ok(())
    }

    /// Record a download/mux failure: transition to `Failed`, increment `retry_count`, and store `last_error`.
    pub async fn record_media_failure(&self, id: i64, err_msg: &str) -> Result<()> {
        let truncated: String = err_msg.chars().take(MAX_ERROR_LEN).collect();
        let res = media::Entity::update_many()
            .col_expr(
                media::Column::State,
                Expr::value(MediaState::Failed.as_str()),
            )
            .col_expr(
                media::Column::RetryCount,
                Expr::col(media::Column::RetryCount).add(1),
            )
            .col_expr(media::Column::LastError, Expr::value(Some(truncated)))
            .col_expr(media::Column::UpdatedAt, Expr::value(Some(now_unix())))
            .filter(media::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown media<{id}>"));
        }
        Ok(())
    }

    /// Reset matching medias back to `Pending`, clearing `retry_count` and `last_error`.
    pub async fn reset_media_states(&self, filter: &MediaResetFilter) -> Result<u64> {
        let mut state_list: Vec<&str> = Vec::new();
        if filter.failed {
            state_list.push(MediaState::Failed.as_str());
        }
        if filter.expired {
            state_list.push(MediaState::Expired.as_str());
        }
        if filter.permission_denied {
            state_list.push(MediaState::PermissionDenied.as_str());
        }
        if filter.skipped {
            state_list.push(MediaState::Skipped.as_str());
        }
        if filter.completed {
            state_list.push(MediaState::Completed.as_str());
        }

        if !filter.all && filter.ids.is_empty() && state_list.is_empty() {
            return Ok(0);
        }

        let now = now_unix();
        let txn = self.conn.begin().await?;
        let mut total = 0u64;

        if filter.all {
            let res = media::Entity::update_many()
                .col_expr(
                    media::Column::State,
                    Expr::value(MediaState::Pending.as_str()),
                )
                .col_expr(media::Column::RetryCount, Expr::value(0i32))
                .col_expr(
                    media::Column::LastError,
                    Expr::value(Option::<String>::None),
                )
                .col_expr(media::Column::UpdatedAt, Expr::value(Some(now)))
                .exec(&txn)
                .await?;
            total += res.rows_affected;
        } else {
            if !state_list.is_empty() {
                let res = media::Entity::update_many()
                    .col_expr(
                        media::Column::State,
                        Expr::value(MediaState::Pending.as_str()),
                    )
                    .col_expr(media::Column::RetryCount, Expr::value(0i32))
                    .col_expr(
                        media::Column::LastError,
                        Expr::value(Option::<String>::None),
                    )
                    .col_expr(media::Column::UpdatedAt, Expr::value(Some(now)))
                    .filter(media::Column::State.is_in(state_list))
                    .exec(&txn)
                    .await?;
                total += res.rows_affected;
            }
            for chunk in filter.ids.chunks(CHUNK_SIZE) {
                let res = media::Entity::update_many()
                    .col_expr(
                        media::Column::State,
                        Expr::value(MediaState::Pending.as_str()),
                    )
                    .col_expr(media::Column::RetryCount, Expr::value(0i32))
                    .col_expr(
                        media::Column::LastError,
                        Expr::value(Option::<String>::None),
                    )
                    .col_expr(media::Column::UpdatedAt, Expr::value(Some(now)))
                    .filter(media::Column::Id.is_in(chunk.iter().copied()))
                    .exec(&txn)
                    .await?;
                total += res.rows_affected;
            }
        }

        txn.commit().await?;
        Ok(total)
    }

    /// Mark specified media IDs as `Skipped`.
    pub async fn skip_medias(&self, ids: &[i64]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let now = now_unix();
        let txn = self.conn.begin().await?;
        let mut total = 0u64;
        for chunk in ids.chunks(CHUNK_SIZE) {
            let res = media::Entity::update_many()
                .col_expr(
                    media::Column::State,
                    Expr::value(MediaState::Skipped.as_str()),
                )
                .col_expr(
                    media::Column::LastError,
                    Expr::value(Option::<String>::None),
                )
                .col_expr(media::Column::UpdatedAt, Expr::value(Some(now)))
                .filter(media::Column::Id.is_in(chunk.iter().copied()))
                .exec(&txn)
                .await?;
            total += res.rows_affected;
        }
        txn.commit().await?;
        Ok(total)
    }

    pub async fn all_medias(&self) -> Result<Vec<media::Model>> {
        media::Entity::find()
            .order_by_asc(media::Column::Id)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn all_active_medias(&self) -> Result<Vec<media::Model>> {
        media::Entity::find()
            .filter(active_media_condition())
            .order_by_asc(media::Column::Id)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    /// Return active medias that do not yet have any `media_up` metadata link (for incremental fetch).
    pub async fn all_active_medias_without_up(&self) -> Result<Vec<media::Model>> {
        let has_up = Query::select()
            .expr(Expr::val(1))
            .from(media_up::Entity)
            .and_where(
                Expr::col((media_up::Entity, media_up::Column::Id))
                    .equals((media::Entity, media::Column::Id)),
            )
            .to_owned();
        media::Entity::find()
            .filter(
                Condition::all()
                    .add(active_media_condition())
                    .add(Expr::exists(has_up).not()),
            )
            .order_by_asc(media::Column::Id)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn all_active_pending_medias(&self) -> Result<Vec<media::Model>> {
        media::Entity::find()
            .filter(
                Condition::all()
                    .add(media::Column::State.eq(MediaState::Pending))
                    .add(active_media_condition()),
            )
            .order_by_asc(media::Column::Id)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn count_active_medias_in_state(&self, state: MediaState) -> Result<u64> {
        media::Entity::find()
            .filter(
                Condition::all()
                    .add(media::Column::State.eq(state))
                    .add(active_media_condition()),
            )
            .count(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn count_active_pending_medias(&self) -> Result<u64> {
        self.count_active_medias_in_state(MediaState::Pending).await
    }

    pub async fn active_media_ids(&self) -> Result<HashSet<i64>> {
        let ids: Vec<i64> = media::Entity::find()
            .select_only()
            .column(media::Column::Id)
            .filter(active_media_condition())
            .into_tuple()
            .all(&self.conn)
            .await?;
        Ok(ids.into_iter().collect())
    }

    pub async fn count_active_media_ids(&self, ids: &[i64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let unique: Vec<i64> = ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut total = 0usize;
        for chunk in unique.chunks(CHUNK_SIZE) {
            let cnt = media::Entity::find()
                .filter(
                    Condition::all()
                        .add(media::Column::Id.is_in(chunk.iter().copied()))
                        .add(active_media_condition()),
                )
                .count(&self.conn)
                .await?;
            total += cnt as usize;
        }
        Ok(total)
    }

    /// Reset any media rows in `ids` that are currently in `Downloading` state back to `Pending`.
    /// Rows in any other state (e.g. `Completed`, `Failed`, `Expired`) are left untouched.
    pub async fn reset_downloading_medias(&self, ids: &[i64]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let now = now_unix();
        let txn = self.conn.begin().await?;
        let mut total = 0u64;
        for chunk in ids.chunks(CHUNK_SIZE) {
            let res = media::Entity::update_many()
                .col_expr(
                    media::Column::State,
                    Expr::value(MediaState::Pending.as_str()),
                )
                .col_expr(
                    media::Column::LastError,
                    Expr::value(Option::<String>::None),
                )
                .col_expr(media::Column::UpdatedAt, Expr::value(Some(now)))
                .filter(
                    Condition::all()
                        .add(media::Column::Id.is_in(chunk.iter().copied()))
                        .add(media::Column::State.eq(MediaState::Downloading.as_str())),
                )
                .exec(&txn)
                .await?;
            total += res.rows_affected;
        }
        txn.commit().await?;
        Ok(total)
    }

    /// Return `true` if `id` belongs to an `Active` set owned by or an `Active` UP followed by
    /// at least one currently-`Active` account in `account_ids`.
    pub async fn is_media_active_for_any_account(
        &self,
        id: i64,
        account_ids: &[i64],
    ) -> Result<bool> {
        if account_ids.is_empty() {
            return Ok(false);
        }
        let found = media::Entity::find()
            .filter(
                Condition::all()
                    .add(media::Column::Id.eq(id))
                    .add(active_media_condition_for_accounts(Some(account_ids))),
            )
            .one(&self.conn)
            .await?;
        Ok(found.is_some())
    }

    pub async fn is_media_active(&self, id: i64) -> Result<bool> {
        let found = media::Entity::find()
            .filter(
                Condition::all()
                    .add(media::Column::Id.eq(id))
                    .add(active_media_condition()),
            )
            .one(&self.conn)
            .await?;
        Ok(found.is_some())
    }

    pub async fn completed_medias(&self, active_only: bool) -> Result<Vec<media::Model>> {
        let mut query = media::Entity::find()
            .filter(media::Column::State.eq(MediaState::Completed))
            .order_by_asc(media::Column::Id);
        if active_only {
            query = query.filter(active_media_condition());
        }
        query.all(&self.conn).await.map_err(Into::into)
    }

    pub async fn query_medias(&self, filter: &MediaFilter) -> Result<Vec<media::Model>> {
        let mut query = media::Entity::find();
        if !filter.states.is_empty() {
            let states: Vec<String> = filter.states.iter().map(|s| s.to_string()).collect();
            query = query.filter(media::Column::State.is_in(states));
        }
        if filter.active_only {
            query = query.filter(active_media_condition());
        }
        if let Some(set_id) = filter.set_id {
            let sub = Query::select()
                .column(media_set::Column::Id)
                .from(media_set::Entity)
                .and_where(Expr::col(media_set::Column::SetId).eq(set_id))
                .to_owned();
            query = query.filter(media::Column::Id.in_subquery(sub));
        }
        if let Some(up_id) = filter.up_id {
            let sub = Query::select()
                .column(media_up::Column::Id)
                .from(media_up::Entity)
                .and_where(Expr::col(media_up::Column::UpId).eq(up_id))
                .to_owned();
            query = query.filter(media::Column::Id.in_subquery(sub));
        }
        if let Some(account_id) = filter.account_id {
            query = query.filter(account_media_condition(account_id));
        }
        if let Some(media_type) = &filter.media_type
            && !media_type.trim().is_empty()
        {
            query = query.filter(Expr::cust_with_values(
                "LOWER(type) = ?",
                [media_type.trim().to_ascii_lowercase()],
            ));
        }
        if let Some(search) = &filter.search
            && !search.trim().is_empty()
        {
            let pattern = format!("%{}%", search.trim().to_ascii_lowercase());
            query = query.filter(Expr::cust_with_values(
                "(LOWER(title) LIKE ? OR LOWER(bv_id) LIKE ?)",
                [pattern.clone(), pattern],
            ));
        }

        query = match (filter.sort, filter.desc) {
            (MediaSort::Id, false) => query.order_by_asc(media::Column::Id),
            (MediaSort::Id, true) => query.order_by_desc(media::Column::Id),
            (MediaSort::Title, false) => query.order_by_asc(media::Column::Title),
            (MediaSort::Title, true) => query.order_by_desc(media::Column::Title),
            (MediaSort::State, false) => query.order_by_asc(media::Column::State),
            (MediaSort::State, true) => query.order_by_desc(media::Column::State),
            (MediaSort::UpdatedAt, false) => query.order_by_asc(media::Column::UpdatedAt),
            (MediaSort::UpdatedAt, true) => query.order_by_desc(media::Column::UpdatedAt),
        };

        if let Some(limit) = filter.limit {
            query = query.limit(limit);
        }

        query.all(&self.conn).await.map_err(Into::into)
    }

    /// Query media rows eligible for `fav pull` with optional target filters.
    /// Always enforces active-media scoping (active account + active set/UP), scoped to a single
    /// active account when `filter.account_id` is set, regardless of whether `ids`, `set_ids`, or
    /// `up_ids` filters are supplied.
    ///
    /// Dimension filters (`ids`, `set_ids`, `up_ids`) combine with AND across dimensions and OR
    /// within each dimension.
    pub async fn pull_candidates(&self, filter: &PullFilter) -> Result<Vec<media::Model>> {
        let active_cond = match filter.account_id {
            Some(account_id) => active_media_condition_for_account(account_id),
            None => active_media_condition(),
        };
        let mut query = media::Entity::find()
            .filter(active_cond)
            .order_by_asc(media::Column::Id);

        if !filter.ids.is_empty() {
            query = query.filter(media::Column::Id.is_in(filter.ids.iter().copied()));
        }
        if !filter.set_ids.is_empty() {
            let sub = Query::select()
                .column(media_set::Column::Id)
                .from(media_set::Entity)
                .and_where(
                    Expr::col(media_set::Column::SetId).is_in(filter.set_ids.iter().copied()),
                )
                .to_owned();
            query = query.filter(media::Column::Id.in_subquery(sub));
        }
        if !filter.up_ids.is_empty() {
            let sub = Query::select()
                .column(media_up::Column::Id)
                .from(media_up::Entity)
                .and_where(Expr::col(media_up::Column::UpId).is_in(filter.up_ids.iter().copied()))
                .to_owned();
            query = query.filter(media::Column::Id.in_subquery(sub));
        }

        if !filter.force {
            let mut allowed = vec![
                MediaState::Pending.as_str(),
                MediaState::Downloading.as_str(),
            ];
            if filter.retry_failed {
                allowed.push(MediaState::Failed.as_str());
            }
            query = query.filter(media::Column::State.is_in(allowed));
        } else if filter.ids.is_empty() {
            let mut allowed = vec![
                MediaState::Pending.as_str(),
                MediaState::Downloading.as_str(),
                MediaState::Completed.as_str(),
            ];
            if filter.retry_failed {
                allowed.push(MediaState::Failed.as_str());
            }
            query = query.filter(media::Column::State.is_in(allowed));
        }

        if let Some(limit) = filter.limit {
            query = query.limit(limit);
        }

        query.all(&self.conn).await.map_err(Into::into)
    }

    /// Prune orphaned sets, inactive medias, and unreferenced UPs atomically in dependency order:
    /// `sets -> medias -> ups`. Preserves `Completed` medias unless `purge_completed` is `true`.
    /// When `dry_run` is `true`, rolls back the transaction and returns exact predicted row counts.
    pub async fn prune_all(&self, purge_completed: bool, dry_run: bool) -> Result<PruneReport> {
        let txn = self.conn.begin().await?;

        let sets_pruned = txn
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                PRUNE_SETS_SQL,
            ))
            .await?
            .rows_affected();

        let medias_sql = if purge_completed {
            PRUNE_MEDIAS_PURGE_COMPLETED_SQL
        } else {
            PRUNE_MEDIAS_PRESERVE_COMPLETED_SQL
        };
        let medias_pruned = txn
            .execute(Statement::from_string(DatabaseBackend::Sqlite, medias_sql))
            .await?
            .rows_affected();

        let ups_pruned = txn
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                PRUNE_UPS_SQL,
            ))
            .await?
            .rows_affected();

        if dry_run {
            txn.rollback().await?;
        } else {
            txn.commit().await?;
        }

        Ok(PruneReport {
            sets_pruned,
            medias_pruned,
            ups_pruned,
            dry_run,
        })
    }

    /// Cleanup inactive/orphan medias (preserving `Completed` unless `purge_completed` is `true`).
    pub async fn prune_medias(&self, purge_completed: bool) -> Result<u64> {
        let sql = if purge_completed {
            PRUNE_MEDIAS_PURGE_COMPLETED_SQL
        } else {
            PRUNE_MEDIAS_PRESERVE_COMPLETED_SQL
        };
        let res = self
            .conn
            .execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
            .await?;
        Ok(res.rows_affected())
    }

    /// Aggregate counts across accounts, sets, UPs, medias, and top UPs for `fav status`.
    pub async fn status_summary(&self, top_n: usize) -> Result<StatusSummary> {
        let mut summary = StatusSummary::default();

        for row in account::Entity::find().all(&self.conn).await? {
            summary.accounts.total += 1;
            match row.state.as_str() {
                s if s == AccountState::Active.as_str() => summary.accounts.active += 1,
                s if s == AccountState::Inactive.as_str() => summary.accounts.inactive += 1,
                s if s == AccountState::Expired.as_str() => summary.accounts.expired += 1,
                _ => {}
            }
        }

        for row in set::Entity::find().all(&self.conn).await? {
            summary.sets.total += 1;
            match row.state.as_str() {
                s if s == SetState::Active.as_str() => summary.sets.active += 1,
                s if s == SetState::Inactive.as_str() => summary.sets.inactive += 1,
                s if s == SetState::Unreachable.as_str() => summary.sets.unreachable += 1,
                _ => {}
            }
        }

        for row in up::Entity::find().all(&self.conn).await? {
            summary.ups.total += 1;
            match row.state.as_str() {
                s if s == UpState::Active.as_str() => summary.ups.active += 1,
                s if s == UpState::Inactive.as_str() => summary.ups.inactive += 1,
                s if s == UpState::Deactivated.as_str() => summary.ups.deactivated += 1,
                _ => {}
            }
        }

        let media_rows = self
            .conn
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT state, COUNT(*) AS cnt FROM media GROUP BY state;",
            ))
            .await?;
        for row in media_rows {
            let state: String = row.try_get("", "state")?;
            let cnt: i64 = row.try_get("", "cnt")?;
            let cnt = cnt.max(0) as u64;
            summary.medias.total += cnt;
            match state.as_str() {
                s if s == MediaState::Pending.as_str() => summary.medias.pending += cnt,
                s if s == MediaState::Downloading.as_str() => summary.medias.downloading += cnt,
                s if s == MediaState::Completed.as_str() => summary.medias.completed += cnt,
                s if s == MediaState::Failed.as_str() => summary.medias.failed += cnt,
                s if s == MediaState::Expired.as_str() => summary.medias.expired += cnt,
                s if s == MediaState::PermissionDenied.as_str() => {
                    summary.medias.permission_denied += cnt
                }
                s if s == MediaState::Skipped.as_str() => summary.medias.skipped += cnt,
                _ => {}
            }
        }

        if top_n > 0 {
            let top_rows = self
                .conn
                .query_all(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    r#"
SELECT u.up_id AS up_id, u.name AS name, COUNT(mu.id) AS media_count
FROM up u
JOIN media_up mu ON u.up_id = mu.up_id
JOIN media m ON mu.id = m.id
GROUP BY u.up_id, u.name
ORDER BY media_count DESC, u.up_id ASC
LIMIT ?;
"#,
                    [sea_orm::Value::BigInt(Some(top_n as i64))],
                ))
                .await?;
            for row in top_rows {
                summary.top_ups.push(TopUpStat {
                    up_id: row.try_get("", "up_id")?,
                    name: row.try_get("", "name")?,
                    media_count: row.try_get("", "media_count")?,
                });
            }
        }

        Ok(summary)
    }
}

const PRUNE_MEDIAS_PRESERVE_COMPLETED_SQL: &str = r#"
DELETE FROM media
WHERE state != 'Completed'
AND id IN (
    SELECT m.id
    FROM media m
    WHERE NOT EXISTS (
        SELECT 1 FROM media_up mu
        JOIN up u ON mu.up_id = u.up_id
        WHERE mu.id = m.id AND u.state = 'Active'
    )
    AND NOT EXISTS (
        SELECT 1 FROM media_set ms
        JOIN "set" s ON ms.set_id = s.set_id
        WHERE ms.id = m.id AND s.state = 'Active'
    )
);
"#;

const PRUNE_MEDIAS_PURGE_COMPLETED_SQL: &str = r#"
DELETE FROM media
WHERE id IN (
    SELECT m.id
    FROM media m
    WHERE NOT EXISTS (
        SELECT 1 FROM media_up mu
        JOIN up u ON mu.up_id = u.up_id
        WHERE mu.id = m.id AND u.state = 'Active'
    )
    AND NOT EXISTS (
        SELECT 1 FROM media_set ms
        JOIN "set" s ON ms.set_id = s.set_id
        WHERE ms.id = m.id AND s.state = 'Active'
    )
);
"#;
