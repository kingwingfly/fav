use super::{client, error::Result, parse_message};
use crate::api::error::PullFail;
use crate::cli::utils::download_bar;
use crate::meta::meta;
use crate::proto::data::{Clarity, PlayInfo, VideoMeta};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt as _, BufWriter};
use tracing::warn;

const API: &str = "https://api.bilibili.com/x/player/playurl";

struct PullOption {
    title: String,
    bvid: String,
    cid: i64,
    clarity: Clarity,
}

impl From<VideoMeta> for PullOption {
    fn from(value: VideoMeta) -> Self {
        Self {
            title: value.title,
            bvid: value.bvid,
            cid: value.cid,
            clarity: value.clarity.unwrap(),
        }
    }
}

pub(crate) async fn pull() {
    let videos: Vec<_> = meta().videos.iter().filter(|v| v.track).collect();
    let mut meta = meta().clone();
    for batch in videos.chunks(10) {
        let jhs: Vec<_> = batch
            .into_iter()
            .filter(|v| v.track)
            .map(|&v| tokio::spawn(do_pull(v.clone().into())))
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
}

async fn do_pull(opt: PullOption) -> Result<String> {
    let url = reqwest::Url::parse_with_params(
        API,
        [
            ("bvid", opt.bvid.as_str()),
            ("cid", &opt.cid.to_string()),
            ("qn", &opt.clarity.to_qn()),
        ],
    )
    .unwrap();
    let resp = client().get(url).send().await?;
    let json: serde_json::Value = resp.json().await?;
    match json["code"].as_i64().unwrap() {
        0 => {
            let mut play_info: PlayInfo = parse_message(&json["data"]["durl"][0]);
            play_info.title = opt.title;
            download(play_info).await?;
            Ok(opt.bvid)
        }
        _ => PullFail {
            msg: format!("Pull Fail bvid:{}; Expired or other reason.", opt.bvid),
        }
        .fail(),
    }
}

async fn download(play_info: PlayInfo) -> Result<()> {
    let PlayInfo {
        url, title, size, ..
    } = play_info;
    let pb = download_bar(size);
    let mut resp = client().get(url).send().await?;
    let mut file = BufWriter::new(File::create(format!("{title}.mp4")).await.unwrap());
    while let Some(chunk) = resp.chunk().await.unwrap() {
        pb.inc(chunk.len() as u64);
        file.write_all(&chunk).await.unwrap();
    }
    file.flush().await.unwrap();
    pb.finish();
    Ok(())
}

impl Clarity {
    fn to_qn(&self) -> String {
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
    async fn pull_test() {
        let opt = PullOption {
            title: "test2".to_string(),
            bvid: "BV1mM4y1C7Kc".to_string(),
            cid: 1093879241,
            clarity: Clarity::FourK,
        };
        do_pull(opt).await.unwrap();
    }
}
