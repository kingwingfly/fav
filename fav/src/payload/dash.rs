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
#[api_req(path = "/x/player/wbi/playurl")]
pub struct DashPayload {
    pub avid: i64, // Do not change the field order
    pub cid: i64,
    pub fnval: u16,
    pub fourk: u8,
    pub qn: u16,
    pub wts: u64,
    #[serde(flatten)]
    pub wbi: Option<WbiEncoded>,
}

impl DashPayload {
    pub async fn new(avid: i64, cid: i64) -> Result<Self> {
        let mut this = Self {
            avid,
            cid,
            fnval: 16 | 64 | 128 | 1024,
            fourk: 1,
            qn: 127,
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
