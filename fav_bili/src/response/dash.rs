use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
pub struct DashResp {
    pub data: DashData,
}

#[derive(Debug, Deserialize)]
pub struct DashData {
    pub dash: Dash,
}

#[derive(Debug, Deserialize)]
pub struct Dash {
    pub video: Vec<Video>,
    pub audio: Vec<Audio>,
}

#[derive(Debug, Deserialize)]
pub struct Video {
    pub base_url: Url,
}

#[derive(Debug, Deserialize)]
pub struct Audio {
    pub base_url: Url,
}
