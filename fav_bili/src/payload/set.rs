use anyhow::Result;
use api_req::{ApiCaller as _, Payload};
use serde::Serialize;

use crate::{
    api::BiliApi,
    response::{WbiData, WbiResp},
    wbi::{WbiEncoded, WbiEncoder},
};

use super::WbiPayload;

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

#[derive(Debug, Payload, Serialize)]
#[api_req(path = "/x/space/wbi/arc/search")]
pub struct InUpPayload {
    pub mid: i64, // Do not change the field order
    pub pn: i64,
    pub ps: u8,
    pub wts: u64,
    #[serde(flatten)]
    pub wbi: Option<WbiEncoded>,
}

impl InUpPayload {
    pub async fn new(mid: i64, pn: i64, ps: u8) -> Result<Self> {
        let mut this = Self {
            mid,
            pn,
            ps,
            wts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            wbi: None,
        };
        let WbiResp {
            data: WbiData { wbi_img, .. },
        } = BiliApi::request(WbiPayload).await?;
        this.wbi = Some(WbiEncoder::encode(wbi_img, &this));
        Ok(this)
    }
}
