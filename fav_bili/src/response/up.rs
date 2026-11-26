use serde::Deserialize;

use super::null_as_default;

#[derive(Debug, Clone, Deserialize)]
pub struct Up {
    pub mid: i64,
    #[serde(alias = "uname")]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FollowingUpData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<Up>,
    #[serde(default)]
    pub total: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FollowingNumData {
    pub following: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishNumData {
    pub video: i64,
}
