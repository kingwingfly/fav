use api_req::{Method, Payload};
use serde::Serialize;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/passport-login/web/qrcode/generate")]
pub struct QrPayload;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/passport-login/web/qrcode/poll")]
pub struct QrPollPayload {
    pub qrcode_key: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/login/exit/v2", method = Method::POST, req = form)]
pub struct LogoutPayload {
    pub biliCSRF: String,
}
