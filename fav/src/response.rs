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
