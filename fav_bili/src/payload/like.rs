use std::fmt;

use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct LikePayload {
    pub aid: i64,
    /// `1` = like, `2` = unlike.
    pub like: u8,
    pub csrf: String,
}

impl LikePayload {
    pub fn like(aid: i64, csrf: String) -> Self {
        Self { aid, like: 1, csrf }
    }

    pub fn unlike(aid: i64, csrf: String) -> Self {
        Self { aid, like: 2, csrf }
    }
}

impl fmt::Debug for LikePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LikePayload")
            .field("aid", &self.aid)
            .field("like", &self.like)
            .field("csrf", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_payload_redacts_csrf() {
        let payload = LikePayload::unlike(12345, "secret_jct".to_string());
        assert_eq!(payload.like, 2);
        let dbg = format!("{payload:?}");
        assert!(!dbg.contains("secret_jct"));
        assert!(dbg.contains("[REDACTED]"));
    }
}
