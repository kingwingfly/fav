use crate::{
    proto::bili::{Bili, BiliSets},
    utils::bar::download_bar,
    FavUtilsError,
};
use fav_core::prelude::*;
use reqwest::header::CONTENT_LENGTH;
use std::io::{BufWriter, Write as _};

impl PathInfo for Bili {
    #[cfg(test)]
    const PATH: &'static str = "temp/temp.bili";
    #[cfg(not(test))]
    const PATH: &'static str = ".fav/bili";
}

impl PathInfo for BiliSets {
    #[cfg(test)]
    const PATH: &'static str = "temp/temp.sets";
    #[cfg(not(test))]
    const PATH: &'static str = ".fav/sets";
}

impl SaveLocal for Bili {
    async fn download<R: Res>(&self, res: &mut R, urls: Vec<reqwest::Url>) -> FavCoreResult<()> {
        let title = res.title();
        let mut urls = urls;
        let mut resp_v = self.client().get(urls.pop().unwrap()).send().await?;
        let mut resp_a = self.client().get(urls.pop().unwrap()).send().await?;
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
                    return Err(FavCoreError::Cancel)
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
        res.on_status(StatusFlags::SAVED);
        Ok(())
    }
}

async fn merge(title: &str, path_v: &str, path_a: &str) -> FavCoreResult<()> {
    let status = tokio::process::Command::new("ffmpeg")
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
        false => Err(FavCoreError::UtilsError(Box::new(
            FavUtilsError::MergeFailed,
        ))),
    }
}
