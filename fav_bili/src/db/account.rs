use anyhow::{Context as _, Result, anyhow};
use sea_orm::{
    ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _, QueryOrder as _,
    QuerySelect as _, TransactionTrait as _,
    sea_query::{Expr, OnConflict, SimpleExpr},
};

use super::Db;
use crate::{entity::account, state::AccountState};

#[derive(Debug, Clone, Default)]
pub struct AccountFilter {
    pub states: Vec<AccountState>,
    pub search: Option<String>,
    pub limit: Option<u64>,
}

impl Db {
    /// Insert or update an account, refreshing `name`, `cookies`, `state`, and `refresh_token`.
    pub async fn upsert_account(&self, account: account::Model) -> Result<()> {
        account::Entity::insert(account.into_active_model())
            .on_conflict(
                OnConflict::column(account::Column::AccountId)
                    .update_columns([
                        account::Column::Name,
                        account::Column::Cookies,
                        account::Column::State,
                        account::Column::RefreshToken,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(&self.conn)
            .await?;
        Ok(())
    }

    /// Update stored cookies and optionally `refresh_token` for an existing account.
    pub async fn update_account_cookies(
        &self,
        account_id: i64,
        cookies: String,
        refresh_token: Option<String>,
    ) -> Result<()> {
        let mut query = account::Entity::update_many()
            .col_expr(account::Column::Cookies, Expr::value(cookies))
            .filter(account::Column::AccountId.eq(account_id));
        if refresh_token.is_some() {
            query = query.col_expr(account::Column::RefreshToken, Expr::value(refresh_token));
        }
        let res = query.exec(&self.conn).await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown account<{account_id}>"));
        }
        Ok(())
    }

    pub async fn set_account_state(&self, account_id: i64, state: AccountState) -> Result<()> {
        let res = account::Entity::update_many()
            .col_expr(account::Column::State, Expr::value(state.as_str()))
            .filter(account::Column::AccountId.eq(account_id))
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown account<{account_id}>"));
        }
        Ok(())
    }

    pub async fn get_account(&self, account_id: i64) -> Result<account::Model> {
        account::Entity::find_by_id(account_id)
            .one(&self.conn)
            .await?
            .context(format!("Unknown account<{account_id}>"))
    }

    pub async fn all_accounts(&self) -> Result<Vec<account::Model>> {
        account::Entity::find()
            .order_by_asc(account::Column::AccountId)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn active_accounts(&self) -> Result<Vec<account::Model>> {
        account::Entity::find()
            .filter(account::Column::State.eq(AccountState::Active))
            .order_by_asc(account::Column::AccountId)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn get_accounts_filtered(&self, filter: SimpleExpr) -> Result<Vec<account::Model>> {
        account::Entity::find()
            .filter(filter)
            .order_by_asc(account::Column::AccountId)
            .all(&self.conn)
            .await
            .map_err(Into::into)
    }

    pub async fn query_accounts(&self, filter: &AccountFilter) -> Result<Vec<account::Model>> {
        let mut query = account::Entity::find().order_by_asc(account::Column::AccountId);
        if !filter.states.is_empty() {
            let states: Vec<String> = filter.states.iter().map(|s| s.to_string()).collect();
            query = query.filter(account::Column::State.is_in(states));
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

    pub async fn delete_account(&self, account_id: i64) -> Result<()> {
        let res = account::Entity::delete_by_id(account_id)
            .exec(&self.conn)
            .await?;
        if res.rows_affected == 0 {
            return Err(anyhow!("Unknown account<{account_id}>"));
        }
        Ok(())
    }

    pub async fn activate_account(&self, account_id: i64) -> Result<()> {
        self.set_account_state(account_id, AccountState::Active)
            .await
    }

    pub async fn activate_accounts(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for &id in ids {
            let res = account::Entity::update_many()
                .col_expr(
                    account::Column::State,
                    Expr::value(AccountState::Active.as_str()),
                )
                .filter(account::Column::AccountId.eq(id))
                .exec(&txn)
                .await?;
            if res.rows_affected == 0 {
                return Err(anyhow!("Unknown account<{id}>"));
            }
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn activate_all_accounts(&self) -> Result<u64> {
        let res = account::Entity::update_many()
            .col_expr(
                account::Column::State,
                Expr::value(AccountState::Active.as_str()),
            )
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }

    pub async fn deactivate_account(&self, account_id: i64) -> Result<()> {
        self.set_account_state(account_id, AccountState::Inactive)
            .await
    }

    pub async fn deactivate_accounts(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for &id in ids {
            let res = account::Entity::update_many()
                .col_expr(
                    account::Column::State,
                    Expr::value(AccountState::Inactive.as_str()),
                )
                .filter(account::Column::AccountId.eq(id))
                .exec(&txn)
                .await?;
            if res.rows_affected == 0 {
                return Err(anyhow!("Unknown account<{id}>"));
            }
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn deactivate_all_accounts(&self) -> Result<u64> {
        let res = account::Entity::update_many()
            .col_expr(
                account::Column::State,
                Expr::value(AccountState::Inactive.as_str()),
            )
            .exec(&self.conn)
            .await?;
        Ok(res.rows_affected)
    }
}
