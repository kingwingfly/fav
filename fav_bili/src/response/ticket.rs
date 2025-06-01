use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct TicketResp {
    pub data: TicketData,
}

#[derive(Debug, Deserialize)]
pub struct TicketData {
    pub ticket: String,
    pub created_at: u64,
    pub ttl: u64,
}
