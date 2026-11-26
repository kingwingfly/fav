use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct LikeResp {
    pub code: i64,
    #[serde(default)]
    pub message: String,
}
