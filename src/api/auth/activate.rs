use crate::{
    api::{auth::utils::murmur3_x64_128, client, error::Result, timestamp},
    proto::data::Cookie,
};
use rand::Rng;
use reqwest::header::COOKIE;
use std::io::Cursor;
use tracing::{info, warn};

use super::utils::webgl_str;

const BUVID_API: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const ACTIVE_API: &str = "https://api.bilibili.com/x/internal/gaia-gateway/ExClimbWuzhi";

#[derive(serde::Deserialize)]
struct Buvids {
    #[serde(rename = "b_3")]
    buvid3: String,
    #[serde(rename = "b_4")]
    buvid4: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Payload {
    #[serde(rename = "payload")]
    inner: String,
}

impl Payload {
    fn new() -> Self {
        let mut rng = rand::thread_rng();
        let mut inner: serde_json::Value =
            serde_json::from_str(include_str!("payload.json")).unwrap();
        inner["5062"] = timestamp().to_string().into();
        inner["6e7c"] =
            format!("{}x{}", rng.gen_range(800..1200), rng.gen_range(1200..3000)).into();
        inner["3c43"]["bfe9"] = webgl_str().into();
        inner["d02f"] = format!("124.0434{:010}", rng.gen::<usize>() % 1e10 as usize).into();
        Payload {
            inner: inner.to_string(),
        }
    }
}

/// Activate the buvid, add `buvid3` `buvid4` `_uuid` `buvid_fp` to cookie.
pub(super) async fn activate_buvid(cookie: &mut Cookie) -> Result<()> {
    let resp = client().get(BUVID_API).send().await?;
    let mut json: serde_json::Value = resp.json().await?;
    let buvids: Buvids = serde_json::from_value(json["data"].take()).unwrap();
    Buvids {
        buvid3: cookie.buvid3,
        buvid4: cookie.buvid4,
    } = buvids;

    cookie._uuid = uuid();

    let payload = Payload::new();
    cookie.buvid_fp = buvid_fp(&payload.inner);

    let resp = client()
        .post(ACTIVE_API)
        .header(
            COOKIE,
            format!(
                "buvid3={}; buvid4={}; _uuid={}; buvid_fp={};",
                cookie.buvid3, cookie.buvid4, cookie._uuid, cookie.buvid_fp,
            ),
        )
        .json(&payload)
        .send()
        .await?;
    let json: serde_json::Value = resp.json().await?;
    #[cfg(test)]
    println!("Activate buvid json = {:#?}", json);
    match json["code"].as_i64().unwrap() {
        0 => info!("Actived Buvid."),
        _ => {
            #[cfg(test)]
            println!("Activate Buvid Failed");
            warn!("Failed to active Buvid. Error Message: {}", json["message"]);
        }
    }
    Ok(())
}

fn uuid() -> String {
    const LEN: usize = 16;
    const DIGHT_MAP: [&str; LEN] = [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F", "10",
    ];
    let t = timestamp() % 100_000;
    let mut rng = rand::thread_rng();
    let index: [u8; 32] = rng.gen();
    let mut result = String::with_capacity(64);
    index.into_iter().enumerate().for_each(|(ii, i)| {
        if [9, 13, 17, 21].contains(&ii) {
            result.push('-')
        };
        result.push_str(DIGHT_MAP[(i & 0x0f) as usize]);
    });
    format!("{}{:0>5}{}", result, t, "infoc")
}

/// https://github.com/SocialSisterYi/bilibili-API-collect/issues/933#issue-2073916390
fn buvid_fp(payload: &str) -> String {
    let tmp: u128 = murmur3_x64_128(&mut Cursor::new(payload), 31);
    format!("{:016x}{:016x}", tmp & (u64::MAX as u128), tmp >> 64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config;

    #[tokio::test]
    async fn active_buvid_test() {
        let mut cookie = config().cookie.clone();
        assert!(activate_buvid(&mut cookie).await.is_ok());
        cookie.persist();
    }

    #[test]
    fn uuid_test() {
        println!("{}", uuid());
    }

    #[test]
    fn buvid_fp_test() {
        let payload = Payload::new();
        println!("{}", buvid_fp(&payload.inner));
    }
}
