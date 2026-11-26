use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, Mac as _};
use serde::Serialize;
use sha2::Sha256;

const TICKET_HMAC_KEY: &str = "XgwSnGZ1p";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Serialize)]
pub struct TicketPayload {
    pub key_id: String,
    pub hexsign: String,
    #[serde(rename(serialize = "context[ts]"))]
    pub context_ts: u64,
    pub csrf: String,
}

impl TicketPayload {
    pub fn new(csrf: String) -> Self {
        let context_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self::with_timestamp(csrf, context_ts)
    }

    pub fn with_timestamp(csrf: String, context_ts: u64) -> Self {
        let mut mac =
            HmacSha256::new_from_slice(TICKET_HMAC_KEY.as_bytes()).expect("HMAC accepts any key");
        mac.update(format!("ts{context_ts}").as_bytes());
        let hexsign = hex::encode(mac.finalize().into_bytes());
        Self {
            key_id: "ec02".to_string(),
            hexsign,
            context_ts,
            csrf,
        }
    }
}

impl fmt::Debug for TicketPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TicketPayload")
            .field("key_id", &self.key_id)
            .field("hexsign", &self.hexsign)
            .field("context_ts", &self.context_ts)
            .field("csrf", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_payload_hmac_is_deterministic_and_redacts_csrf() {
        let p = TicketPayload::with_timestamp("secret_csrf".to_string(), 1_700_000_000);
        assert_eq!(p.key_id, "ec02");
        assert_eq!(p.hexsign.len(), 64);
        // Verified against HMAC-SHA256(key="XgwSnGZ1p", msg="ts1700000000")
        assert_eq!(
            p.hexsign,
            "bb79f0d980ffbb51597aa1a3e8b55603025cc1322ac766f4c1a98852e6182514"
        );
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("secret_csrf"));
        assert!(dbg.contains("[REDACTED]"));
    }
}
