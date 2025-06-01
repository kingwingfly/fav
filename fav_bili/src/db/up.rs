use anyhow::{Context as _, Result};
use sea_orm::{
    ActiveValue::{Set, Unchanged},
    ConnectionTrait as _, EntityTrait as _, IntoActiveModel as _, Value,
    sea_query::{OnConflict, SimpleExpr},
};
use sea_orm::{DatabaseBackend, Statement};

use super::Db;
use crate::entity::up;
use crate::state::UpState;

impl Db {
    pub async fn upsert_ups(&self, ups: impl IntoIterator<Item = up::Model>) -> Result<()> {
        up::Entity::insert_many(ups.into_iter().map(|s| s.into_active_model()))
            .on_conflict(
                OnConflict::column(up::Column::UpId)
                    .update_columns([up::Column::Name])
                    .to_owned(),
            )
            .exec_without_returning(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_up(&self, up_id: i64) -> Result<up::Model> {
        up::Entity::find_by_id(up_id)
            .one(&self.db)
            .await?
            .context(format!("Unknown up<{}>", up_id))
    }

    pub async fn all_ups(&self) -> Result<Vec<up::Model>> {
        up::Entity::find().all(&self.db).await.map_err(Into::into)
    }

    pub async fn activate_up(&self, up_id: i64) -> Result<()> {
        up::Entity::update(up::ActiveModel {
            up_id: Unchanged(up_id),
            state: Set(UpState::Active.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn activate_all_ups(&self) -> Result<()> {
        up::Entity::update_many()
            .col_expr(
                up::Column::State,
                SimpleExpr::Value(Value::String(Some(Box::new(UpState::Active.to_string())))),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn deactivate_up(&self, up_id: i64) -> Result<()> {
        up::Entity::update(up::ActiveModel {
            up_id: Unchanged(up_id),
            state: Set(UpState::Inactive.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn deactivate_all_ups(&self) -> Result<()> {
        up::Entity::update_many()
            .col_expr(
                up::Column::State,
                SimpleExpr::Value(Value::String(Some(Box::new(UpState::Inactive.to_string())))),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Cleanup the ups followd by no account
    pub async fn prune_ups(&self) -> Result<()> {
        self.db
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                r#"
DELETE FROM up
WHERE up_id IN (
    SELECT up_id
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
"#,
            ))
            .await?;
        Ok(())
    }
}
