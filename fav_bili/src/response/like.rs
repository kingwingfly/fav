use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LikeResp {
    pub code: i64,
    pub message: String,
}
