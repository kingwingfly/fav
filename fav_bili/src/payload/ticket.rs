use api_req::{Method, Payload};
use ring::hmac::{HMAC_SHA256, Key, sign};
use serde::Serialize;

const KEY: &str = "XgwSnGZ1p";

#[derive(Debug, Serialize, Payload)]
#[api_req(path =
    "/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket",
    method = Method::POST,
    req = query,
)]
pub struct TicketPayload {
    key_id: String,
    hexsign: String,
    #[serde(rename(serialize = "context[ts]"))]
    context_ts: u64,
    csrf: String,
}

impl TicketPayload {
    pub fn new(csrf: String) -> Self {
        let context_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let key = Key::new(HMAC_SHA256, KEY.as_bytes());
        let tag = sign(&key, format!("ts{context_ts}").as_bytes());
        let hexsign = hex::encode(tag);
        Self {
            key_id: "ec02".to_string(),
            hexsign,
            context_ts,
            csrf,
        }
    }
}
