use std::{io::Write as _, sync::Arc};

use anyhow::{Result, anyhow};
use api_req::ApiCaller as _;
use avmux::{AVFile, Mux as _};
use dashmap::DashSet;
use futures::StreamExt as _;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use reqwest::header::{CONTENT_LENGTH, HeaderValue};
use sea_orm::ColumnTrait as _;
use tempfile::NamedTempFile;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    api::BiliApi,
    cookies::{add_cookie_jar, parse_cookies},
    db::{Db, db},
    entity::{account, media},
    payload::{DashPayload, MediaInfoPayload},
    response::{Dash, DashData, DashResp, MediaInfoData, MediaInfoResp, Page},
    state::{AccountState, MediaState},
    table::head,
};

const BAR_TEMPLATE: &str = "{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})";

pub async fn pull() -> Result<()> {
    let db = db().await;
    let accounts = db
        .get_accounts_filtered(account::Column::State.eq(AccountState::Active))
        .await?;
    let pulled_medias = Arc::new(DashSet::<i64>::new());
    let medias = db.all_active_pending_medias().await?;
    let bars = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
    for account in accounts {
        info!("Pulling medias with account<{}>", account.name);
        add_cookie_jar(parse_cookies(&account.cookies));
        let token = CancellationToken::new();
        let mut tasks = futures::stream::iter(
            medias
                .iter()
                .filter(|media| !pulled_medias.contains(&media.id)),
        )
        .map(|media| {
            let token = token.clone();
            let db = db.clone();
            let bars = bars.clone();
            let pulled_medias = pulled_medias.clone();
            async move {
                tokio::select! {
                    res = download(media, db, bars), if !token.is_cancelled() => match res {
                        Ok(_) => { pulled_medias.insert(media.id); }
                        Err(e) => error!("{}", e),
                    },
                    _ = token.cancelled() => {},
                }
            }
        })
        .buffer_unordered(8);
        loop {
            tokio::select! {
                res = tasks.next() => {
                    if res.is_none() {
                        break;
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    token.cancel();
                    warn!("Received Ctrl-C");
                    break;
                }
            }
        }
    }
    drop(bars);
    info!("Finished pulling");
    Ok(())
}

async fn download(media: &media::Model, db: Db, bars: MultiProgress) -> Result<()> {
    match BiliApi::request(MediaInfoPayload { aid: media.id }).await? {
        MediaInfoResp {
            data: Some(MediaInfoData { pages, .. }),
            code: 0,
            ..
        } => {
            let only1p = pages.len() == 1;
            for Page { cid, page, part } in pages {
                let filename = if only1p {
                    format!("{}-{}", media.id, media.title)
                } else {
                    format!("{}-{}({page})-{part}", media.id, media.title)
                };
                let DashResp {
                    data:
                        DashData {
                            dash: Dash { video, audio },
                        },
                } = BiliApi::request(DashPayload::new(media.id, cid).await?).await?;

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
                        pb.set_message(head(part, 10));
                        pb.set_style(
                            ProgressStyle::with_template(BAR_TEMPLATE)
                                .unwrap()
                                .progress_chars("#>-"),
                        );
                        let mut file_v = NamedTempFile::new()?;
                        let mut file_a = NamedTempFile::new()?;
                        let (mut finished_v, mut finished_a) = (false, false);
                        loop {
                            tokio::select! {
                                res = resp_v.chunk(), if !finished_v => {
                                    match res {
                                        Ok(Some(chunk)) => {
                                            file_v.write_all(&chunk)?;
                                            file_v.flush()?;
                                            pb.inc(chunk.len() as u64);
                                        }
                                        Ok(None) => finished_v = true,
                                        Err(e) => return Err(anyhow!(
                                            "Failed to download video {filename}: {e}"
                                        ))
                                    }
                                }
                                res = resp_a.chunk(), if !finished_a => {
                                    match res {
                                        Ok(Some(chunk)) => {
                                            file_a.write_all(&chunk)?;
                                            file_a.flush()?;
                                            pb.inc(chunk.len() as u64);
                                        }
                                        Ok(None) => finished_a = true,
                                        Err(e) => return Err(anyhow!(
                                            "Failed to download audio {filename}: {e}"
                                        ))
                                    }
                                }
                                else => break,
                            }
                        }
                        let title = format!(
                            "{filename}.mp4",
                            filename = sanitize_filename::sanitize(&filename)
                        );
                        let output_path = format!("./{}", title);
                        [
                            AVFile::new(file_v.path().to_string_lossy()),
                            AVFile::new(file_a.path().to_string_lossy()),
                        ]
                        .mux(AVFile::new(&output_path))
                        .map_err(|e| {
                            std::fs::remove_file(output_path).ok();
                            anyhow!("Failed to mux video and audio {filename}: {e:?}")
                        })?;
                    }
                    (Some(v), None) => {
                        let mut resp_v = BiliApi::client().get(v.base_url).send().await?;
                        let hv2u64 =
                            |hv: &HeaderValue| -> u64 { hv.to_str().unwrap().parse().unwrap() };
                        let size = hv2u64(&resp_v.headers()[CONTENT_LENGTH]);
                        let pb = ProgressBar::new(size);
                        bars.add(pb.clone());
                        pb.set_message(head(part, 10));
                        pb.set_style(
                            ProgressStyle::with_template(BAR_TEMPLATE)
                                .unwrap()
                                .progress_chars("#>-"),
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
                                    return Err(anyhow!(
                                        "Failed to download video {filename}: {e}"
                                    ));
                                }
                            }
                        }
                        let title = format!(
                            "{filename}.mp4",
                            filename = sanitize_filename::sanitize(&filename)
                        );
                        tokio::fs::rename(file_v.path(), format!("./{}", title)).await?;
                    }
                    (None, Some(a)) => {
                        let mut resp_a = BiliApi::client().get(a.base_url).send().await?;
                        let hv2u64 =
                            |hv: &HeaderValue| -> u64 { hv.to_str().unwrap().parse().unwrap() };
                        let size = hv2u64(&resp_a.headers()[CONTENT_LENGTH]);
                        let pb = ProgressBar::new(size);
                        bars.add(pb.clone());
                        pb.set_message(head(part, 10));
                        pb.set_style(
                            ProgressStyle::with_template(BAR_TEMPLATE)
                                .unwrap()
                                .progress_chars("#>-"),
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
                                    return Err(anyhow!(
                                        "Failed to download audio {filename}: {e}"
                                    ));
                                }
                            }
                        }
                        let title = format!(
                            "{filename}.mp3",
                            filename = sanitize_filename::sanitize(&filename)
                        );
                        tokio::fs::rename(file_a.path(), format!("./{}", title)).await?;
                    }
                    _ => {}
                }
            }
            db.set_media_state(media.id, MediaState::Completed).await?;
            Ok(())
        }
        MediaInfoResp {
            code,
            message: option_msg,
            ..
        } => {
            db.set_media_state(
                media.id,
                match code {
                    -403 | 62012 | 62002 => MediaState::PermissionDenied,
                    _ => MediaState::Expired,
                },
            )
            .await?;
            Err(anyhow!(
                "Info unreachable media<{}-{}>: {}",
                media.id,
                media.title,
                option_msg.unwrap_or_default()
            ))
        }
    }
}
