use api_req::{ApiCaller, Payload};
use serde::Serialize;

use crate::{
    api::BiliApi,
    response::{WbiData, WbiResp},
    wbi::{WbiEncoded, WbiEncoder},
};

use super::WbiPayload;

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/space/wbi/acc/info")]
pub struct WhoamiPayload {
    mid: u64,
    wts: u64,
    #[serde(flatten)]
    wbi: Option<WbiEncoded>,
}

impl WhoamiPayload {
    pub async fn new(mid: u64) -> Self {
        let mut payload = Self {
            mid,
            wts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            wbi: None,
        };
        let WbiResp {
            data: WbiData { wbi_img },
        } = BiliApi::request(WbiPayload).await.unwrap();
        payload.wbi = Some(WbiEncoder::encode(wbi_img, &payload));
        payload
    }
}
