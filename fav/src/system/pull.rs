use std::io::Write;

use anyhow::{Result, anyhow};
use api_req::ApiCaller as _;
use bevy_ecs::{
    observer::Trigger,
    system::{Commands, Res},
};
use dashmap::DashSet;
use futures::StreamExt as _;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use reqwest::header::{CONTENT_LENGTH, HeaderValue};
use tempfile::NamedTempFile;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    api::BiliApi,
    cookies::{parse_cookies, set_cookie_jar},
    db::Db,
    event::Pull,
    payload::{DashPayload, MediaInfoPayload},
    response::{Dash, DashData, DashResp, MediaInfoData, MediaInfoResp, Page},
    runtime::Runtime,
    state::MediaState,
    table::head,
};

pub fn pull(mut cmds: Commands) {
    cmds.add_observer(|_: Trigger<Pull>, runtime: Res<Runtime>, db: Res<Db>| {
        if let Err(e) = runtime.block_on(async {
            let accounts = db.all_active_accounts().await?;
            let pulled_medias = DashSet::<i64>::new();
            let medias = db.all_pending_medias().await?;
            let bars = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
            for account in accounts {
                info!("Pulling medias with account<{}>", account.name);
                set_cookie_jar(parse_cookies(&account.cookies));
                let token = CancellationToken::new();
                let mut tasks = futures::stream::iter(
                    medias
                        .iter()
                        .filter(|media| !pulled_medias.contains(&media.id)),
                )
                .take(1)
                .map(|media| {
                    let token = token.clone();
                    let db = db.clone();
                    let bars = bars.clone();
                    async move {
                        if token.is_cancelled() {
                            return Ok::<_, anyhow::Error>(());
                        }
                        tokio::select! {
                            res = download(media.id, db, bars) => res,
                            _ = token.cancelled() => Ok(()),
                        }
                    }
                })
                .buffer_unordered(8);
                loop {
                    tokio::select! {
                        res = tasks.next() => {
                            match res {
                                Some(res) => if let Err(e) = res {
                                    error!("{}", e);
                                }
                                None => break,
                            }
                        }
                        _ = tokio::signal::ctrl_c() => {
                            token.cancel();
                            break;
                        }
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        }) {
            error!("{}", e);
        }
    });
}

async fn download(avid: i64, db: Db, bars: MultiProgress) -> Result<()> {
    match BiliApi::request(MediaInfoPayload { aid: avid }).await? {
        MediaInfoResp {
            data: Some(MediaInfoData { pages, .. }),
            code: 0,
            ..
        } => {
            'a: for Page { cid, page, part } in pages {
                let DashResp {
                    data:
                        DashData {
                            dash: Dash { video, audio },
                        },
                } = BiliApi::request(DashPayload::new(avid, cid).await?).await?;

                match (video.into_iter().next(), audio.into_iter().next()) {
                    (Some(v), Some(a)) => {
                        let mut resp_v = BiliApi::client().get(v.base_url).send().await?;
                        let mut resp_a = BiliApi::client().get(a.base_url).send().await?;
                        let hv2u64 =
                            |hv: &HeaderValue| -> u64 { hv.to_str().unwrap().parse().unwrap() };
                        let size = hv2u64(&resp_v.headers()[CONTENT_LENGTH])
                            + hv2u64(&resp_a.headers()[CONTENT_LENGTH]);
                        let pb = ProgressBar::new(size);
                        bars.add(pb.clone());
                        pb.set_message(head(part.to_owned(), 10));
                        pb.set_style(
                            ProgressStyle::with_template("{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                                .unwrap()
                                .progress_chars("#>-")
                        );
                        let mut file_v = NamedTempFile::new()?;
                        let mut file_a = NamedTempFile::new()?;
                        loop {
                            tokio::select! {
                                Ok(Some(chunk)) = resp_v.chunk() => {
                                    file_v.write_all(&chunk)?;
                                    file_v.flush()?;
                                    pb.inc(chunk.len() as u64);
                                }
                                Ok(Some(chunk)) = resp_a.chunk() => {
                                    file_a.write_all(&chunk)?;
                                    file_a.flush()?;
                                    pb.inc(chunk.len() as u64);
                                }
                                else => break,
                            }
                        }
                        let mut title = sanitize_filename::sanitize(&part);
                        title.push_str(&format!(" {}({}).mp4", avid, page));
                        let status = tokio::process::Command::new("ffmpeg")
                            .args([
                                "-y",
                                "-i",
                                file_v.path().to_str().unwrap(),
                                "-i",
                                file_a.path().to_str().unwrap(),
                                "-codec",
                                "copy",
                                "-f",
                                "mp4",
                                &format!("./{}.mp4", title),
                            ])
                            .stderr(std::process::Stdio::null())
                            .status()
                            .await
                            .unwrap();
                        if !status.success() {
                            return Err(anyhow!("Failed to merge video and audio part<{}>", part));
                        }
                    }
                    (Some(v), None) => {
                        let mut resp_v = BiliApi::client().get(v.base_url).send().await?;
                        let hv2u64 =
                            |hv: &HeaderValue| -> u64 { hv.to_str().unwrap().parse().unwrap() };
                        let size = hv2u64(&resp_v.headers()[CONTENT_LENGTH]);
                        let pb = ProgressBar::new(size);
                        bars.add(pb.clone());
                        pb.set_message(head(part.to_owned(), 10));
                        pb.set_style(
                            ProgressStyle::with_template("{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                                .unwrap()
                                .progress_chars("#>-")
                        );
                        let mut file_v = NamedTempFile::new()?;
                        loop {
                            match resp_v.chunk().await {
                                Ok(Some(chunk)) => {
                                    file_v.write_all(&chunk)?;
                                    file_v.flush()?;
                                    pb.inc(chunk.len() as u64);
                                }
                                Ok(None) => break,
                                Err(e) => {
                                    error!("{}", e);
                                    continue 'a;
                                }
                            }
                        }
                        let mut title = sanitize_filename::sanitize(&part);
                        title.push_str(&format!(" {}({}).mp4", avid, page));
                        tokio::fs::rename(file_v.path(), format!("./{}", title)).await?;
                    }
                    (None, Some(a)) => {
                        let mut resp_a = BiliApi::client().get(a.base_url).send().await?;
                        let hv2u64 =
                            |hv: &HeaderValue| -> u64 { hv.to_str().unwrap().parse().unwrap() };
                        let size = hv2u64(&resp_a.headers()[CONTENT_LENGTH]);
                        let pb = ProgressBar::new(size);
                        bars.add(pb.clone());
                        pb.set_message(head(part.to_owned(), 10));
                        pb.set_style(
                            ProgressStyle::with_template("{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                                .unwrap()
                                .progress_chars("#>-")
                        );
                        let mut file_a = NamedTempFile::new()?;
                        loop {
                            match resp_a.chunk().await {
                                Ok(Some(chunk)) => {
                                    file_a.write_all(&chunk)?;
                                    file_a.flush()?;
                                    pb.inc(chunk.len() as u64);
                                }
                                Ok(None) => break,
                                Err(e) => {
                                    error!("{}", e);
                                    continue 'a;
                                }
                            }
                        }
                        let mut title = sanitize_filename::sanitize(&part);
                        title.push_str(&format!(" {}({}).mp3", avid, page));
                        tokio::fs::rename(file_a.path(), format!("./{}", title)).await?;
                    }
                    _ => {}
                }
            }
            db.set_media_state(avid, MediaState::Completed).await?;
            Ok(())
        }
        MediaInfoResp {
            code,
            message: option_msg,
            ..
        } => {
            db.set_media_state(
                avid,
                match code {
                    -403 | 62012 | 62002 => MediaState::PermissionDenied,
                    _ => MediaState::Expired,
                },
            )
            .await?;
            Err(anyhow!(
                "Info unreachable media<{}>: {}",
                avid,
                option_msg.unwrap_or_default()
            ))
        }
    }
}
