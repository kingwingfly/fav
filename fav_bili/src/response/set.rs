use serde::Deserialize;

use super::Media;

#[derive(Debug, Deserialize)]
pub struct ListSetResp {
    pub data: ListSetData,
}

#[derive(Debug, Deserialize)]
pub struct ListSetData {
    pub list: Vec<Set>,
}

#[derive(Debug, Deserialize)]
pub struct Set {
    pub id: i64,
    pub media_count: i64,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct InSetResp {
    pub data: InSetData,
}

#[derive(Debug, Deserialize)]
pub struct InSetData {
    pub medias: Vec<Media>,
}

#[derive(Debug, Deserialize)]
pub struct InUpResp {
    pub data: InUpData,
}

#[derive(Debug, Deserialize)]
pub struct InUpData {
    pub list: InUpList,
}

#[derive(Debug, Deserialize)]
pub struct InUpList {
    pub vlist: Vec<Media>,
}
