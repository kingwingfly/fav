use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Up {
    pub mid: i64,
    #[serde(alias = "uname")]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct FollowingUpResp {
    pub data: FollowingUpData,
}

#[derive(Debug, Deserialize)]
pub struct FollowingUpData {
    pub list: Vec<Up>,
}

#[derive(Debug, Deserialize)]
pub struct FollowingNumResp {
    pub data: FollowingNumData,
}

#[derive(Debug, Deserialize)]
pub struct FollowingNumData {
    pub following: i64,
}
