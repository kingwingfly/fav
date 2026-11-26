use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
pub struct QrData {
    pub qrcode_key: String,
    pub url: Url,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QrPollData {
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogoutResp {
    pub code: i64,
    #[serde(default, alias = "msg")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CookieInfoData {
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub timestamp: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CookieRefreshData {
    #[serde(default)]
    pub status: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}
