//! `SeaORM` Entity, updated for additive schema enhancements.

use std::fmt;

use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Copy, Clone, Default, Debug, DeriveEntity)]
pub struct Entity;

impl EntityName for Entity {
    fn table_name(&self) -> &str {
        "account"
    }
}

#[derive(Clone, PartialEq, DeriveModel, DeriveActiveModel, Eq, Serialize)]
pub struct Model {
    pub account_id: i64,
    pub name: String,
    #[serde(skip_serializing)]
    pub cookies: String,
    pub state: String,
    #[serde(skip_serializing)]
    pub refresh_token: Option<String>,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("account_id", &self.account_id)
            .field("name", &self.name)
            .field("cookies", &"[REDACTED]")
            .field("state", &self.state)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
pub enum Column {
    AccountId,
    Name,
    Cookies,
    State,
    RefreshToken,
}

#[derive(Copy, Clone, Debug, EnumIter, DerivePrimaryKey)]
pub enum PrimaryKey {
    AccountId,
}

impl PrimaryKeyTrait for PrimaryKey {
    type ValueType = i64;
    fn auto_increment() -> bool {
        false
    }
}

#[derive(Copy, Clone, Debug, EnumIter)]
pub enum Relation {
    SetAccount,
    UpAccount,
}

impl ColumnTrait for Column {
    type EntityName = Entity;
    fn def(&self) -> ColumnDef {
        match self {
            Self::AccountId => ColumnType::BigInteger.def(),
            Self::Name => ColumnType::String(StringLen::None).def(),
            Self::Cookies => ColumnType::String(StringLen::None).def(),
            Self::State => ColumnType::custom("enum_text").def(),
            Self::RefreshToken => ColumnType::String(StringLen::None).def().null(),
        }
    }
}

impl RelationTrait for Relation {
    fn def(&self) -> RelationDef {
        match self {
            Self::SetAccount => Entity::has_many(super::set_account::Entity).into(),
            Self::UpAccount => Entity::has_many(super::up_account::Entity).into(),
        }
    }
}

impl Related<super::set_account::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SetAccount.def()
    }
}

impl Related<super::up_account::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UpAccount.def()
    }
}

impl Related<super::set::Entity> for Entity {
    fn to() -> RelationDef {
        super::set_account::Relation::Set.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::set_account::Relation::Account.def().rev())
    }
}

impl Related<super::up::Entity> for Entity {
    fn to() -> RelationDef {
        super::up_account::Relation::Up.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::up_account::Relation::Account.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
