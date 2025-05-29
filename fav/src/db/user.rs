use anyhow::{Context as _, Result};
use fav::migration::OnConflict;
use sea_orm::{
    ActiveValue::{Set, Unchanged},
    EntityTrait, IntoActiveModel as _,
};

use super::Db;
use crate::{entity::user, state::UserState};

impl Db {
    pub async fn upsert_user(&self, user: user::Model) -> Result<()> {
        user::Entity::insert(user.into_active_model())
            .on_conflict(
                OnConflict::column(user::Column::UserId)
                    .update_columns([user::Column::Name, user::Column::Cookies])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn get_user(&self, user_id: i32) -> Result<user::Model> {
        user::Entity::find_by_id(user_id)
            .one(&self.db)
            .await?
            .context(format!("Unknown user_id<{}>", user_id))
    }

    pub async fn delete_user(&self, user_id: i32) -> Result<()> {
        user::Entity::delete_by_id(user_id).exec(&self.db).await?;
        Ok(())
    }

    pub async fn all_users(&self) -> Result<Vec<user::Model>> {
        user::Entity::find().all(&self.db).await.map_err(Into::into)
    }

    pub async fn delete_all(&self) -> Result<()> {
        user::Entity::delete_many().exec(&self.db).await?;
        Ok(())
    }

    pub async fn activate(&self, user_id: i32) -> Result<()> {
        user::Entity::update(user::ActiveModel {
            user_id: Unchanged(user_id),
            state: Set(UserState::Active.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn deactivate(&self, user_id: i32) -> Result<()> {
        user::Entity::update(user::ActiveModel {
            user_id: Unchanged(user_id),
            state: Set(UserState::Inactive.to_string()),
            ..Default::default()
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }
}
