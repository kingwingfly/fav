use serde::Deserialize;

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
    pub title: String,
}
