use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FollowingUpPayload {
    pub vmid: i64,
    pub pn: i64,
    pub ps: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct FollowingNumPayload {
    pub vmid: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishNumPayload {
    pub mid: i64,
}
