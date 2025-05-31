use anyhow::Result;
use sea_orm::{ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _};

use super::Db;
use crate::entity::up_account;

impl Db {
    pub async fn upsert_up_account(&self, up_account: up_account::Model) -> Result<()> {
        up_account::Entity::insert(up_account.into_active_model())
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn upsert_up_accounts(
        &self,
        up_accounts: impl IntoIterator<Item = up_account::Model>,
    ) -> Result<()> {
        up_account::Entity::insert_many(up_accounts.into_iter().map(|m| m.into_active_model()))
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_up_ids_of_account(&self, account_id: i64) -> Result<Vec<i64>> {
        up_account::Entity::find()
            .filter(up_account::Column::AccountId.eq(account_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.up_id).collect())
    }

    pub async fn get_account_ids_of_up(&self, up_id: i64) -> Result<Vec<i64>> {
        up_account::Entity::find()
            .filter(up_account::Column::UpId.eq(up_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.account_id).collect())
    }

    pub async fn delete_up_account(&self, up_account: up_account::Model) -> Result<()> {
        up_account::Entity::delete_by_id((up_account.up_id, up_account.account_id))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
