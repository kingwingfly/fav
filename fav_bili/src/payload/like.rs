use api_req::Payload;
use reqwest::Method;
use serde::Serialize;

#[derive(Debug, Serialize, Payload)]
#[api_req(
    path = "/x/web-interface/archive/like",
    method = Method::POST,
    req = form
)]
pub struct LikePayload {
    pub aid: i64,
    pub like: u8,
    pub csrf: String,
}
