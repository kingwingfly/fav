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

    pub async fn get_user(&self, user_id: i32) -> user::Model {
        user::Entity::find_by_id(user_id)
            .one(&self.db)
            .await
            .unwrap()
            .unwrap()
    }

    pub async fn delete_user(&self, user_id: i32) {
        user::Entity::delete_by_id(user_id)
            .exec(&self.db)
            .await
            .unwrap();
    }

    pub async fn all_users(&self) -> Vec<user::Model> {
        user::Entity::find().all(&self.db).await.unwrap()
    }

    pub async fn delete_all(&self) {
        user::Entity::delete_many().exec(&self.db).await.unwrap();
    }
}
