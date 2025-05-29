use anyhow::{Context as _, Result};
use fav::migration::OnConflict;
use sea_orm::{
    ActiveValue::{Set, Unchanged},
    ColumnTrait as _, EntityTrait as _, IntoActiveModel as _, QueryFilter as _,
};

use super::Db;
use crate::{
    entity::set,
    state::{AccountState, SetState},
};

impl Db {
    pub async fn upsert_set(&self, set: set::Model) -> Result<()> {
        set::Entity::insert(set.into_active_model())
            .on_conflict(
                OnConflict::column(set::Column::SetId)
                    .update_columns([set::Column::Name])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_set(&self, set_id: i64) -> Result<set::Model> {
        set::Entity::find_by_id(set_id)
            .one(&self.db)
            .await?
            .context(format!("Unknown set<{}>", set_id))
    }

    pub async fn delete_set(&self, set_id: i64) -> Result<()> {
        set::Entity::delete_by_id(set_id).exec(&self.db).await?;
        Ok(())
    }

    pub async fn all_sets(&self) -> Result<Vec<set::Model>> {
        set::Entity::find().all(&self.db).await.map_err(Into::into)
    }

    pub async fn all_active_sets(&self) -> Result<Vec<set::Model>> {
        set::Entity::find()
            .filter(set::Column::State.eq(SetState::Active.to_string()))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn activate_set(&self, set_id: i64) -> Result<()> {
        set::Entity::update(set::ActiveModel {
            set_id: Unchanged(set_id),
            state: Set(AccountState::Active.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn deactivate_set(&self, set_id: i64) -> Result<()> {
        set::Entity::update(set::ActiveModel {
            set_id: Unchanged(set_id),
            state: Set(AccountState::Inactive.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }
}
