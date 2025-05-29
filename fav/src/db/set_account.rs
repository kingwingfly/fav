use anyhow::Result;
use sea_orm::{ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _};

use super::Db;
use crate::entity::set_account;

impl Db {
    pub async fn upsert_set_account(&self, set_account: set_account::Model) -> Result<()> {
        set_account::Entity::insert(set_account.into_active_model())
            .on_conflict_do_nothing()
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_sets_of_account(&self, account_id: i64) -> Result<Vec<set_account::Model>> {
        set_account::Entity::find()
            .filter(set_account::Column::AccountId.eq(account_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn get_accounts_of_set(&self, set_id: i64) -> Result<Vec<set_account::Model>> {
        set_account::Entity::find()
            .filter(set_account::Column::SetId.eq(set_id))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn delete_set_account(&self, set_account: set_account::Model) -> Result<()> {
        set_account::Entity::delete_by_id((set_account.set_id, set_account.account_id))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
