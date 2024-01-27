use crate::config::config;
use reqwest::header::COOKIE;
use tracing::{info, warn};

const LIKE_API: &str = "https://api.bilibili.com/x/web-interface/archive/like";

pub(crate) async fn like(bvid: String) {
    let cookie = &config().cookie;
    let url = reqwest::Url::parse_with_params(
        LIKE_API,
        [
            ("bvid", bvid.as_str()),
            ("like", "1"),
            ("csrf", cookie.bili_jct.as_str()),
        ],
    )
    .unwrap();

    let resp = reqwest::Client::new()
        .post(url)
        .header(
            COOKIE,
            format!("SESSDATA={}; buvid3={}", cookie.SESSDATA, cookie.buvid3),
        )
        .send()
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    match json.pointer("/code").unwrap().as_i64().unwrap() {
        0 | 65006 => info!("liked bvid{}", bvid),
        _ => warn!(
            "failed to like bvid:{}; Error Message, {}",
            bvid,
            json.pointer("/message").unwrap()
        ),
    }
}

pub(crate) async fn like_all() {}
