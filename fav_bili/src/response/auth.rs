use serde::Deserialize;
use url::Url;

use super::pick_optional_str;

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
#[serde(from = "RawLogoutResp")]
pub struct LogoutResp {
    pub code: i64,
    pub message: Option<String>,
}

#[derive(Deserialize)]
struct RawLogoutResp {
    code: i64,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

impl From<RawLogoutResp> for LogoutResp {
    fn from(raw: RawLogoutResp) -> Self {
        Self {
            code: raw.code,
            message: pick_optional_str(raw.message, raw.msg),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logout_resp_handles_dual_message_and_msg_keys() {
        let both = r#"{"code": 0, "message": "ok", "msg": "legacy ok"}"#;
        let resp: LogoutResp = serde_json::from_str(both).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(resp.message.as_deref(), Some("ok"));

        let empty_message = r#"{"code": -111, "message": "", "msg": "csrf failed"}"#;
        let resp: LogoutResp = serde_json::from_str(empty_message).unwrap();
        assert_eq!(resp.code, -111);
        assert_eq!(resp.message.as_deref(), Some("csrf failed"));

        let msg_only = r#"{"code": 0, "msg": "logged out"}"#;
        let resp: LogoutResp = serde_json::from_str(msg_only).unwrap();
        assert_eq!(resp.message.as_deref(), Some("logged out"));

        let message_only = r#"{"code": 0, "message": "done"}"#;
        let resp: LogoutResp = serde_json::from_str(message_only).unwrap();
        assert_eq!(resp.message.as_deref(), Some("done"));
    }
}
