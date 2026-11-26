use serde::Deserialize;

use super::{Media, null_as_default};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListSetData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<Set>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Set {
    pub id: i64,
    pub media_count: i64,
    pub title: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InSetData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub medias: Vec<Media>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub info: Option<Set>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InUpData {
    #[serde(default)]
    pub list: InUpList,
    #[serde(default)]
    pub page: Option<InUpPage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InUpPage {
    #[serde(default)]
    pub pn: i64,
    #[serde(default)]
    pub ps: i64,
    #[serde(default)]
    pub count: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InUpList {
    #[serde(default, deserialize_with = "null_as_default")]
    pub vlist: Vec<Media>,
}
