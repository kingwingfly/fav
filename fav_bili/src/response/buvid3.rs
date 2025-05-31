use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Buvid3Resp {
    pub data: Buvid3Data,
}

#[derive(Debug, Deserialize)]
pub struct Buvid3Data {
    pub buvid: String,
}
