use anyhow::{Context as _, Result};
use fav::migration::OnConflict;
use sea_orm::{
    ActiveValue::{Set, Unchanged},
    ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _,
};

use super::Db;
use crate::{entity::account, state::AccountState};

impl Db {
    pub async fn upsert_account(&self, account: account::Model) -> Result<()> {
        account::Entity::insert(account.into_active_model())
            .on_conflict(
                OnConflict::column(account::Column::AccountId)
                    .update_columns([account::Column::Name, account::Column::Cookies])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_account(&self, account_id: i64) -> Result<account::Model> {
        account::Entity::find_by_id(account_id)
            .one(&self.db)
            .await?
            .context(format!("Unknown account<{}>", account_id))
    }

    pub async fn delete_account(&self, account_id: i64) -> Result<()> {
        account::Entity::delete_by_id(account_id)
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn all_accounts(&self) -> Result<Vec<account::Model>> {
        account::Entity::find()
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn all_active_accounts(&self) -> Result<Vec<account::Model>> {
        account::Entity::find()
            .filter(account::Column::State.eq(AccountState::Active.to_string()))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn activate_account(&self, account_id: i64) -> Result<()> {
        account::Entity::update(account::ActiveModel {
            account_id: Unchanged(account_id),
            state: Set(AccountState::Active.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn deactivate_account(&self, account_id: i64) -> Result<()> {
        account::Entity::update(account::ActiveModel {
            account_id: Unchanged(account_id),
            state: Set(AccountState::Inactive.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }
}
