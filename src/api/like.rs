use crate::{api::client, config::config, meta::meta};
use reqwest::header::COOKIE;
use tracing::{info, warn};

const LIKE_API: &str = "https://api.bilibili.com/x/web-interface/archive/like";
const QUERY_LIKE_API: &str = "https://api.bilibili.com/x/web-interface/archive/has/like";
const LIKE_INTERVAL: u64 = 3;

pub(crate) async fn like(bvid: &str) {
    match is_liked(bvid).await {
        true => info!("Already liked bvid:{}", bvid),
        false => do_like(bvid).await,
    }
}

pub(crate) async fn like_all() {
    let videos = &meta().videos;
    for v in videos.iter().filter(|v| v.track).map(|v| &v.bvid) {
        like(v).await;
        std::thread::sleep(std::time::Duration::from_secs(LIKE_INTERVAL));
    }
}

#[allow(unused)]
pub(crate) async fn like_all_fast() {
    let meta = meta();
    let jhs: Vec<_> = meta
        .videos
        .iter()
        .filter_map(|v| match v.track {
            true => Some(tokio::spawn(like(&v.bvid))),
            false => None,
        })
        .collect();
    for jh in jhs {
        jh.await.unwrap();
    }
}

async fn do_like(bvid: &str) {
    let cookie = &config().cookie;
    let url = reqwest::Url::parse_with_params(
        LIKE_API,
        [
            ("bvid", bvid),
            ("like", "1"),
            ("csrf", cookie.bili_jct.as_str()),
        ],
    )
    .unwrap();

    let resp = client()
        .post(url)
        .header(
            COOKIE,
            format!("SESSDATA={}; buvid3={}", cookie.SESSDATA, cookie.buvid3,),
        )
        .send()
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    match json["code"].as_i64().unwrap() {
        0 => info!("Liked bvid:{}", bvid),
        65006 => info!("Already liked bvid:{}", bvid),
        _ => warn!(
            "Failed to like bvid:{}; Error Message, {}",
            bvid, json["message"]
        ),
    }
}

async fn is_liked(bvid: &str) -> bool {
    let url = reqwest::Url::parse_with_params(QUERY_LIKE_API, [("bvid", bvid)]).unwrap();
    let resp = client().get(url).send().await.unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    json["data"].as_i64().unwrap() == 1
}
