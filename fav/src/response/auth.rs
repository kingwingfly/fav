use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
pub struct QrResp {
    pub data: QrData,
}

#[derive(Debug, Deserialize)]
pub struct QrData {
    pub qrcode_key: String,
    pub url: Url,
}

#[derive(Debug, Deserialize)]
pub struct QrPollResp {
    pub data: QrPollData,
}

#[derive(Debug, Deserialize)]
pub struct QrPollData {
    pub code: u32,
    pub message: String,
}
