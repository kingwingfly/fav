use api_req::Payload;
use serde::Serialize;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/web-interface/nav")]
pub struct WbiPayload;
