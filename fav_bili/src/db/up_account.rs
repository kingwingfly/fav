use anyhow::Result;
use sea_orm::{
    ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _,
    TransactionTrait as _,
};

use super::{CHUNK_SIZE, Db};
use crate::entity::up_account;

impl Db {
    pub async fn upsert_up_accounts(
        &self,
        up_accounts: impl IntoIterator<Item = up_account::Model>,
    ) -> Result<()> {
        let items: Vec<up_account::Model> = up_accounts.into_iter().collect();
        if items.is_empty() {
            return Ok(());
        }
        let txn = self.conn.begin().await?;
        for chunk in items.chunks(CHUNK_SIZE) {
            up_account::Entity::insert_many(chunk.iter().cloned().map(|m| m.into_active_model()))
                .on_conflict_do_nothing()
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_up_ids_of_account(&self, account_id: i64) -> Result<Vec<i64>> {
        up_account::Entity::find()
            .filter(up_account::Column::AccountId.eq(account_id))
            .all(&self.conn)
            .await
            .map_err(Into::into)
            .map(|res| res.into_iter().map(|m| m.up_id).collect())
    }

    pub async fn delete_up_account(&self, up_account: up_account::Model) -> Result<()> {
        up_account::Entity::delete_by_id((up_account.up_id, up_account.account_id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }
}
