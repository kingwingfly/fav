use super::wbi::{encode_wbi, get_wbi_keys};
use super::{client, error::Result, parse_message};
use crate::api::error::PullFail;
use crate::cli::utils::download_bar;
use crate::meta::meta;
use crate::proto::data::{Clarity, PlayInfo, VideoMeta};
use std::io::{BufWriter, Write as _};
use tracing::warn;

const API: &str = "https://api.bilibili.com/x/player/wbi/playurl";

pub(crate) async fn pull_all() {
    let videos = meta().videos.iter().filter(|v| v.track).collect();
    try_pull(videos).await;
}

pub(crate) async fn pull(id: Vec<String>) {
    let videos = id
        .into_iter()
        .flat_map(|id| {
            meta().videos.iter().filter(move |v| {
                (v.track && v.list_ids.contains(&id.parse().unwrap_or_default())) || v.bvid == id
            })
        })
        .collect();
    try_pull(videos).await;
}

async fn try_pull(videos: Vec<&'static VideoMeta>) {
    let mut meta = meta().clone();
    for batch in videos.chunks(10) {
        let jhs: Vec<_> = batch
            .into_iter()
            .filter(|v| v.track && !v.saved)
            .map(|&v| tokio::spawn(do_pull(v)))
            .collect();
        for jh in jhs {
            match jh.await.unwrap() {
                Ok(bvid) => {
                    meta.videos
                        .iter_mut()
                        .find(|v| v.bvid == bvid)
                        .unwrap()
                        .saved = true;
                }
                Err(e) => warn!("{}", e),
            }
        }
    }
    meta.persist();
}

async fn do_pull(video: &VideoMeta) -> Result<String> {
    let wbi_keys = get_wbi_keys().await?;
    let mut params = vec![
        ("bvid", video.bvid.clone()),
        ("cid", video.cid.to_string()),
        ("qn", video.clarity.unwrap().to_qn()),
        ("fnval", "129".to_string()),
        ("fourk", "1".to_string()),
    ];
    let params = encode_wbi(&mut params, wbi_keys);
    let url = reqwest::Url::parse(&format!("{}?{}", API, params)).unwrap();
    let resp = client().get(url).send().await?;
    let json: serde_json::Value = resp.json().await?;
    match json["code"].as_i64().unwrap() {
        0 => {
            let mut play_info: PlayInfo = parse_message(&json["data"]["durl"][0]);
            play_info.title = video.title.to_owned();
            download(play_info).await?;
            Ok(video.bvid.to_owned())
        }
        _ => PullFail {
            msg: format!("Pull Fail bvid:{}; Expired or other reason.", video.bvid),
        }
        .fail(),
    }
}

async fn download(play_info: PlayInfo) -> Result<()> {
    let PlayInfo {
        url, title, size, ..
    } = play_info;
    let pb = download_bar(size);
    pb.set_message(title.chars().take(10).collect::<String>());
    let mut resp = client().get(url).send().await?;
    let mut file = BufWriter::new(tempfile::NamedTempFile::new()?);
    loop {
        tokio::select! {
            chunk = resp.chunk() => {
                match chunk? {
                    Some(chunk) => {
                        pb.inc(chunk.len() as u64);
                        file.write_all(&chunk).unwrap();
                    }
                    None => break,
                }
            },
            _ = tokio::signal::ctrl_c() => {
                file.into_inner().unwrap().close()?;
                return PullFail {
                    msg: "Download Cancelled; Ctrl-C",
                }.fail();
            }

        }
    }
    file.flush().unwrap();
    file.into_inner().unwrap().persist(format!("{title}.mp4"))?;
    pb.finish();
    Ok(())
}

impl Clarity {
    fn to_qn(self) -> String {
        match self {
            Clarity::FourK => "120",
            Clarity::FullHDHighFrame => "116",
            Clarity::FullHDHighCode => "112",
            Clarity::FullHD => "80",
            Clarity::HDHighFrame => "74",
            Clarity::HD => "64",
            Clarity::SD => "32",
            Clarity::LD => "16",
            Clarity::VLD => "6",
        }
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_json() {
        let wbi_keys = get_wbi_keys().await.unwrap();
        let mut params = vec![
            ("bvid", "BV1NN411F7HE".to_string()),
            ("cid", "1049107766".to_string()),
            ("qn", "127".to_string()),
            ("fnval", (16 | 1024).to_string()),
            ("fourk", "1".to_string()),
        ];
        let params = encode_wbi(&mut params, wbi_keys);
        let url = reqwest::Url::parse(&format!("{}?{}", API, params)).unwrap();
        let resp = client().get(url).send().await.unwrap();
        let json: serde_json::Value = resp.json().await.unwrap();
        println!("{:#?}", json);
    }
}
