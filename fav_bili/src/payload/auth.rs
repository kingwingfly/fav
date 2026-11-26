use std::fmt;

use serde::Serialize;

#[derive(Debug, Default, Clone, Serialize)]
pub struct QrPayload;

#[derive(Debug, Clone, Serialize)]
pub struct QrPollPayload {
    pub qrcode_key: String,
}

#[allow(non_snake_case)]
#[derive(Clone, Serialize)]
pub struct LogoutPayload {
    pub biliCSRF: String,
}

impl fmt::Debug for LogoutPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogoutPayload")
            .field("biliCSRF", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct CookieInfoPayload;

#[derive(Clone, Serialize)]
pub struct CookieRefreshPayload {
    pub csrf: String,
    pub refresh_csrf: String,
    pub source: String,
    pub refresh_token: String,
}

impl fmt::Debug for CookieRefreshPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CookieRefreshPayload")
            .field("csrf", &"[REDACTED]")
            .field("refresh_csrf", &"[REDACTED]")
            .field("source", &self.source)
            .field("refresh_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Serialize)]
pub struct ConfirmRefreshPayload {
    pub csrf: String,
    pub refresh_token: String,
}

impl fmt::Debug for ConfirmRefreshPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfirmRefreshPayload")
            .field("csrf", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_payloads_redact_secrets_in_debug() {
        let logout = LogoutPayload {
            biliCSRF: "super-secret-csrf".to_string(),
        };
        let dbg = format!("{logout:?}");
        assert!(!dbg.contains("super-secret-csrf"));
        assert!(dbg.contains("[REDACTED]"));

        let refresh = CookieRefreshPayload {
            csrf: "c1".to_string(),
            refresh_csrf: "c2".to_string(),
            source: "main_web".to_string(),
            refresh_token: "rt_secret".to_string(),
        };
        let dbg = format!("{refresh:?}");
        assert!(!dbg.contains("rt_secret"));
        assert!(dbg.contains("main_web"));
    }
}
