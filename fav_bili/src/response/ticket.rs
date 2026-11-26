use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct TicketData {
    pub ticket: String,
    pub created_at: u64,
    pub ttl: u64,
}
