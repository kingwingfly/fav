use super::error::MergeFail;
use super::wbi::{encode_wbi, get_wbi_keys};
use super::{client, error::Result, parse_message};
use crate::api::error::PullFail;
use crate::cli::utils::download_bar;
use crate::config::config;
use crate::proto::data::{Dash, Meta, Qn, VideoMeta};
use reqwest::header::CONTENT_LENGTH;
use std::io::{BufWriter, Write as _};
use tracing::warn;

const API: &str = "https://api.bilibili.com/x/player/wbi/playurl";

pub(crate) async fn pull_all() {
    let mut meta = Meta::read();
    // Safety: onlt `try_pull` modify `meta`
    let videos = meta
        .videos
        .iter_mut()
        .filter(|v| v.track && !v.saved && !v.expired)
        .map(|v| v as *mut _)
        .collect();
    try_pull(videos).await;
    meta.persist();
}

pub(crate) async fn pull(id: Vec<String>) {
    let mut meta = Meta::read();
    // Safety: onlt `try_pull` modify `meta`
    let mut videos = vec![];
    for id in id {
        videos.extend(
            meta.videos
                .iter_mut()
                .filter(move |v| {
                    ((v.track && v.list_ids.contains(&id.parse().unwrap_or_default()))
                        || v.bvid == id)
                        && !v.saved
                })
                .map(|v| v as *mut _),
        );
    }
    try_pull(videos).await;
    meta.persist();
}

/// Safety: `videos` is a valid pointer to `VideoMeta`; The caller must ensure the `VideoMeta` is not moved or dropped during the lifetime of the pointer
async fn try_pull(videos: Vec<*mut VideoMeta>) {
    unsafe {
        for batch in videos.chunks(10) {
            let jhs: Vec<_> = batch.iter().map(|v| tokio::spawn(do_pull(&**v))).collect();
            for jh in jhs {
                match jh.await.unwrap() {
                    Ok(bvid) => {
                        let v = videos.iter().find(|v| (***v).bvid == bvid).unwrap();
                        (**v).saved = true;
                    }
                    Err(e) => warn!("{}", e),
                }
            }
        }
    }
}

async fn do_pull(meta: &VideoMeta) -> Result<String> {
    let wbi_keys = get_wbi_keys().await?;
    let mut params = vec![
        ("bvid", meta.bvid.clone()),
        ("cid", meta.cid.to_string()),
        ("qn", meta.clarity.unwrap().into()),
        ("fnval", (16 | 1024).to_string()),
        ("fourk", "1".to_string()),
    ];
    let params = encode_wbi(&mut params, wbi_keys);
    let url = reqwest::Url::parse(&format!("{}?{}", API, params)).unwrap();
    let resp = client().get(url).send().await?;
    let json: serde_json::Value = resp.json().await?;
    match json["code"].as_i64().unwrap() {
        0 => {
            let dash: Dash = parse_message(&json["data"]["dash"]);
            let v_url = match meta.clarity.unwrap() {
                Qn::Default => &dash.video[0].base_url,
                _ => dash
                    .video
                    .iter()
                    .find(|v| v.id <= (meta.clarity.unwrap() as i32))
                    .map(|v| &v.base_url)
                    .unwrap(),
            };
            let a_url = &dash.audio[0].base_url;
            download(&meta.title, v_url, a_url).await?;
            Ok(meta.bvid.to_owned())
        }
        _ => PullFail {
            msg: format!("Pull Fail bvid:{}; Expired or other reason.", meta.bvid),
        }
        .fail(),
    }
}

async fn download(title: &str, v_url: &str, a_url: &str) -> Result<()> {
    let mut resp_v = client().get(v_url).send().await?;
    let mut resp_a = client().get(a_url).send().await?;
    let size = resp_v.headers()[CONTENT_LENGTH]
        .to_str()
        .unwrap()
        .parse::<usize>()
        .unwrap()
        + resp_a.headers()[CONTENT_LENGTH]
            .to_str()
            .unwrap()
            .parse::<usize>()
            .unwrap();
    let pb = download_bar(size);
    pb.set_message(title.chars().take(10).collect::<String>());

    let mut file_v = BufWriter::new(tempfile::NamedTempFile::new()?);
    let mut file_a = BufWriter::new(tempfile::NamedTempFile::new()?);
    let mut finish_v = false;
    let mut finish_a = false;
    loop {
        tokio::select! {
            chunk = resp_v.chunk(), if !finish_v => {
                match chunk? {
                    Some(chunk) => {
                        pb.inc(chunk.len() as u64);
                        file_v.write_all(&chunk).unwrap();
                    }
                    None => finish_v = true,
                }
            },
            chunk = resp_a.chunk(), if !finish_a => {
                match chunk? {
                    Some(chunk) => {
                        pb.inc(chunk.len() as u64);
                        file_a.write_all(&chunk).unwrap();
                    }
                    None => finish_a = true,
                }
            },
            _ = tokio::signal::ctrl_c() => {
                file_v.into_inner().unwrap().close()?;
                file_a.into_inner().unwrap().close()?;
                return PullFail {
                    msg: "Download Cancelled; Ctrl-C",
                }.fail();
            }

        }
        if finish_v && finish_a {
            break;
        }
    }
    file_v.flush().unwrap();
    file_a.flush().unwrap();
    pb.finish();
    merge(
        title,
        file_v.into_inner().unwrap().path().to_str().unwrap(),
        file_a.into_inner().unwrap().path().to_str().unwrap(),
    )
    .await?;
    Ok(())
}

async fn merge(title: &str, path_v: &str, path_a: &str) -> Result<()> {
    let status = tokio::process::Command::new(&config().ffmpeg_path)
        .args([
            "-y",
            "-i",
            path_v,
            "-i",
            path_a,
            "-codec",
            "copy",
            "-f",
            "mp4",
            &format!("./{}.mp4", sanitize_filename::sanitize(title)),
        ])
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    match status.success() {
        true => Ok(()),
        false => MergeFail {
            msg: "Merge with ffmpeg faild.",
        }
        .fail(),
    }
}

impl From<Qn> for String {
    fn from(qn: Qn) -> String {
        (qn as usize).to_string()
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

    #[test]
    fn qn_into() {
        let qn = Qn::FourK;
        let s: String = qn.into();
        assert_eq!(s, "120");
    }

    #[test]
    fn pull_all_test() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(pull_all());
    }

    #[test]
    fn illegal_filename() {
        let title = "-你也想要我的To签qian ming嘛";
        let s = sanitize_filename::sanitize(title);
        std::fs::write(format!("{}.txt", s), "test").unwrap();
        println!("{s}");
    }
}
