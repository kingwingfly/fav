use anyhow::{Context as _, Result};
use sea_orm::{
    ActiveValue::{Set, Unchanged},
    ConnectionTrait as _, DatabaseBackend, EntityTrait as _, IntoActiveModel as _, Statement,
    Value,
    sea_query::{OnConflict, SimpleExpr},
};

use super::Db;
use crate::{entity::set, state::SetState};

impl Db {
    pub async fn upsert_sets(&self, sets: impl IntoIterator<Item = set::Model>) -> Result<()> {
        set::Entity::insert_many(sets.into_iter().map(|s| s.into_active_model()))
            .on_conflict(
                OnConflict::column(set::Column::SetId)
                    .update_columns([set::Column::Name, set::Column::Count])
                    .to_owned(),
            )
            .exec_without_returning(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_set(&self, set_id: i64) -> Result<set::Model> {
        set::Entity::find_by_id(set_id)
            .one(&self.db)
            .await?
            .context(format!("Unknown set<{}>", set_id))
    }

    pub async fn all_sets(&self) -> Result<Vec<set::Model>> {
        set::Entity::find().all(&self.db).await.map_err(Into::into)
    }

    pub async fn activate_set(&self, set_id: i64) -> Result<()> {
        set::Entity::update(set::ActiveModel {
            set_id: Unchanged(set_id),
            state: Set(SetState::Active.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn activate_all_sets(&self) -> Result<()> {
        set::Entity::update_many()
            .col_expr(
                set::Column::State,
                SimpleExpr::Value(Value::String(Some(Box::new(SetState::Active.to_string())))),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn deactivate_set(&self, set_id: i64) -> Result<()> {
        set::Entity::update(set::ActiveModel {
            set_id: Unchanged(set_id),
            state: Set(SetState::Inactive.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn deactivate_all_sets(&self) -> Result<()> {
        set::Entity::update_many()
            .col_expr(
                set::Column::State,
                SimpleExpr::Value(Value::String(Some(Box::new(
                    SetState::Inactive.to_string(),
                )))),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Cleanup the sets belonging to no account
    pub async fn prune_sets(&self) -> Result<()> {
        self.db
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                r#"
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
"#,
            ))
            .await?;
        Ok(())
    }
}
