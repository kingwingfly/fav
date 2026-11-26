//! `SeaORM` Entity, updated for additive schema enhancements.

use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Copy, Clone, Default, Debug, DeriveEntity)]
pub struct Entity;

impl EntityName for Entity {
    fn table_name(&self) -> &str {
        "media"
    }
}

#[derive(Clone, Debug, PartialEq, DeriveModel, DeriveActiveModel, Eq, Serialize)]
pub struct Model {
    pub id: i64,
    pub bv_id: String,
    pub title: String,
    pub r#type: String,
    pub state: String,
    pub retry_count: i32,
    pub last_error: Option<String>,
    pub updated_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
pub enum Column {
    Id,
    BvId,
    Title,
    Type,
    State,
    RetryCount,
    LastError,
    UpdatedAt,
}

#[derive(Copy, Clone, Debug, EnumIter, DerivePrimaryKey)]
pub enum PrimaryKey {
    Id,
}

impl PrimaryKeyTrait for PrimaryKey {
    type ValueType = i64;
    fn auto_increment() -> bool {
        false
    }
}

#[derive(Copy, Clone, Debug, EnumIter)]
pub enum Relation {
    MediaSet,
    MediaUp,
    MediaPage,
}

impl ColumnTrait for Column {
    type EntityName = Entity;
    fn def(&self) -> ColumnDef {
        match self {
            Self::Id => ColumnType::BigInteger.def(),
            Self::BvId => ColumnType::String(StringLen::None).def().unique(),
            Self::Title => ColumnType::String(StringLen::None).def(),
            Self::Type => ColumnType::custom("enum_text").def(),
            Self::State => ColumnType::custom("enum_text").def(),
            Self::RetryCount => ColumnType::Integer.def(),
            Self::LastError => ColumnType::String(StringLen::None).def().null(),
            Self::UpdatedAt => ColumnType::BigInteger.def().null(),
        }
    }
}

impl RelationTrait for Relation {
    fn def(&self) -> RelationDef {
        match self {
            Self::MediaSet => Entity::has_many(super::media_set::Entity).into(),
            Self::MediaUp => Entity::has_many(super::media_up::Entity).into(),
            Self::MediaPage => Entity::has_many(super::media_page::Entity).into(),
        }
    }
}

impl Related<super::media_set::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MediaSet.def()
    }
}

impl Related<super::media_up::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MediaUp.def()
    }
}

impl Related<super::media_page::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MediaPage.def()
    }
}

impl Related<super::set::Entity> for Entity {
    fn to() -> RelationDef {
        super::media_set::Relation::Set.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::media_set::Relation::Media.def().rev())
    }
}

impl Related<super::up::Entity> for Entity {
    fn to() -> RelationDef {
        super::media_up::Relation::Up.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::media_up::Relation::Media.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
