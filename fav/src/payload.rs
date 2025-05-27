use api_req::{Method, Payload};
use serde::Serialize;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/passport-login/web/qrcode/generate")]
pub struct QrPayload;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/passport-login/web/qrcode/poll", req=query)]
pub struct QrPollPayload {
    pub qrcode_key: String,
}
