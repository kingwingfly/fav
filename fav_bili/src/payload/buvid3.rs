use api_req::Payload;
use serde::Serialize;

#[derive(Debug, Serialize, Payload)]
#[api_req(path = "/x/web-frontend/getbuvid")]
pub struct Buvid3Payload;
