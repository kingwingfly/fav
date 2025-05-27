use api_req::Payload;
use serde::Serialize;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/passport-login/web/qrcode/generate")]
pub struct QrPayload;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/passport-login/web/qrcode/poll")]
pub struct QrPollPayload {
    pub qrcode_key: String,
}
