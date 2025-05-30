use anyhow::{Context as _, Result};
use fav::migration::OnConflict;
use sea_orm::{EntityTrait as _, IntoActiveModel as _};

use super::Db;
use crate::entity::up;

impl Db {
    pub async fn upsert_up(&self, up: up::Model) -> Result<()> {
        up::Entity::insert(up.into_active_model())
            .on_conflict(
                OnConflict::column(up::Column::UpId)
                    .update_columns([up::Column::Name])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn upsert_ups(&self, ups: impl IntoIterator<Item = up::Model>) -> Result<()> {
        up::Entity::insert_many(ups.into_iter().map(|s| s.into_active_model()))
            .on_conflict(
                OnConflict::column(up::Column::UpId)
                    .update_columns([up::Column::Name])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_up(&self, up_id: i64) -> Result<up::Model> {
        up::Entity::find_by_id(up_id)
            .one(&self.db)
            .await?
            .context(format!("Unknown up<{}>", up_id))
    }

    pub async fn delete_up(&self, up_id: i64) -> Result<()> {
        up::Entity::delete_by_id(up_id).exec(&self.db).await?;
        Ok(())
    }

    pub async fn all_ups(&self) -> Result<Vec<up::Model>> {
        up::Entity::find().all(&self.db).await.map_err(Into::into)
    }
}
