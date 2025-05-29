use api_req::Payload;
use serde::Serialize;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/v3/fav/folder/created/list-all")]
pub struct ListSetPayload {
    pub up_mid: i64,
}
