use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct WhoamiResp {
    pub data: WhoamiData,
}

#[derive(Debug, Deserialize)]
pub struct WhoamiData {
    pub mid: u64,
    pub name: String,
}
