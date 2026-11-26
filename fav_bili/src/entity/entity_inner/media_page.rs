//! `SeaORM` Entity for `media_page` (per-cid multi-page archival tracking).

use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Copy, Clone, Default, Debug, DeriveEntity)]
pub struct Entity;

impl EntityName for Entity {
    fn table_name(&self) -> &str {
        "media_page"
    }
}

#[derive(Clone, Debug, PartialEq, DeriveModel, DeriveActiveModel, Eq, Serialize)]
pub struct Model {
    pub media_id: i64,
    pub cid: i64,
    pub page: i32,
    pub part: String,
    pub state: String,
    pub local_path: Option<String>,
    pub downloaded_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
pub enum Column {
    MediaId,
    Cid,
    Page,
    Part,
    State,
    LocalPath,
    DownloadedAt,
}

#[derive(Copy, Clone, Debug, EnumIter, DerivePrimaryKey)]
pub enum PrimaryKey {
    MediaId,
    Cid,
}

impl PrimaryKeyTrait for PrimaryKey {
    type ValueType = (i64, i64);
    fn auto_increment() -> bool {
        false
    }
}

#[derive(Copy, Clone, Debug, EnumIter)]
pub enum Relation {
    Media,
}

impl ColumnTrait for Column {
    type EntityName = Entity;
    fn def(&self) -> ColumnDef {
        match self {
            Self::MediaId => ColumnType::BigInteger.def(),
            Self::Cid => ColumnType::BigInteger.def(),
            Self::Page => ColumnType::Integer.def(),
            Self::Part => ColumnType::String(StringLen::None).def(),
            Self::State => ColumnType::String(StringLen::None).def(),
            Self::LocalPath => ColumnType::String(StringLen::None).def().null(),
            Self::DownloadedAt => ColumnType::BigInteger.def().null(),
        }
    }
}

impl RelationTrait for Relation {
    fn def(&self) -> RelationDef {
        match self {
            Self::Media => Entity::belongs_to(super::media::Entity)
                .from(Column::MediaId)
                .to(super::media::Column::Id)
                .into(),
        }
    }
}

impl Related<super::media::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Media.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
