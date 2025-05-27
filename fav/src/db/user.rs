use fav::migration::OnConflict;
use sea_orm::{EntityTrait, IntoActiveModel as _};

use super::Db;
use crate::entity::user;

impl Db {
    pub async fn upsert_user(&self, user: user::Model) {
        user::Entity::insert(user.into_active_model())
            .on_conflict(
                OnConflict::column(user::Column::UserId)
                    .update_columns([user::Column::Name, user::Column::Cookies])
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .unwrap();
    }
}
