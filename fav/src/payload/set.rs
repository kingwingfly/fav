use api_req::Payload;
use serde::Serialize;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/v3/fav/folder/created/list-all")]
pub struct ListSetPayload {
    pub up_mid: i64,
}

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/v3/fav/resource/list")]
pub struct InSetPayload {
    pub media_id: i64,
    pub pn: i64,
    pub ps: u8,
}
