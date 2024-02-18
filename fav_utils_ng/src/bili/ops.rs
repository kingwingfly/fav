use super::api::ApiKind;
use crate::{
    proto::bili::{Bili, BiliRes, BiliSet, BiliSets},
    utils::qr::show_qr_code,
    FavUtilsError, FavUtilsResult,
};
use fav_core::{prelude::*, status::SetStatusExt as _};
use reqwest::Response;
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use url::Url;

const POLL_INTERVAL: u64 = 3;
const EXPIRED_DURATION: u64 = 120;
const HINT: &str = "Never Login";

impl Operations<BiliSets, BiliSet, BiliRes, ApiKind> for Bili {
    async fn login(&mut self) -> FavCoreResult<()> {
        let QrInfo { url, qrcode_key } = self.request_json(ApiKind::Qr, vec![], "/data").await?;
        show_qr_code(url)?;
        for _ in 0..EXPIRED_DURATION / POLL_INTERVAL {
            sleep(Duration::from_secs(POLL_INTERVAL)).await;
            let resp = self
                .request(ApiKind::QrPoll, vec![qrcode_key.clone()])
                .await?;
            if let Ok(cookies) = try_extract_cookie(&resp) {
                self.extend_cookies(cookies);
                return Ok(());
            }
        }
        Err(FavCoreError::UtilsError(Box::new(FavUtilsError::QrExpired)))
    }

    async fn logout(&mut self) -> FavCoreResult<()> {
        let params = vec![self.cookies().get("bili_jct").expect(HINT).to_owned()];
        match self.request_json(ApiKind::Logout, params, "/code").await? {
            0 => Ok(()),
            _ => Err(FavCoreError::UtilsError(Box::new(
                FavUtilsError::LogoutError,
            ))),
        }
    }

    async fn fetch_sets(&self, sets: &mut BiliSets) -> FavCoreResult<()> {
        let params = vec![self.cookies().get("DedeUserID").expect(HINT).to_owned()];
        *sets |= self
            .request_proto(ApiKind::FetchSets, params, "/data")
            .await?;
        Ok(())
    }

    async fn fetch_set(&self, set: &mut BiliSet) -> FavCoreResult<()> {
        let id = set.id.to_string();
        for pn in 1..=set.media_count.saturating_sub(1) / 20 + 1 {
            let pn = pn.to_string();
            let params = vec![id.clone(), pn, "20".to_string()];
            *set |= self
                .request_proto::<BiliSet>(ApiKind::FetchSet, params, "/data")
                .await?
                .with_res_status_on(StatusFlags::FAV);
        }
        Ok(())
    }

    async fn fetch_res(&self, resource: &mut BiliRes) -> FavCoreResult<()> {
        let params = vec![resource.bvid.clone()];
        tokio::select! {
            res = self.request_proto::<BiliRes>(ApiKind::FetchRes, params, "/data") => {
                    *resource |= res?;
                    resource.on_status(StatusFlags::FETCHED);
                },
            _ = tokio::signal::ctrl_c() => {
                return Err(FavCoreError::Cancel);
            }
        }
        Ok(())
    }

    async fn pull(&self, resource: &mut BiliRes) -> FavCoreResult<()> {
        if resource.check_status(StatusFlags::SAVED) {
            return Ok(());
        }
        let Wbi { img_url, sub_url } = self
            .request_json::<Wbi>(ApiKind::Wbi, vec![], "/data/wbi_img")
            .await?;
        let params = vec![
            resource.bvid.clone(),
            resource.cid.to_string(),
            resource.qn.unwrap().into(), // Dash format, no effect
            (16 | 1024).to_string(),
            "1".to_string(),
            sub_url,
            img_url,
        ];
        let Dash { audio, video } = self
            .request_json(ApiKind::Pull, params, "/data/dash")
            .await?;
        self.save(resource, vec![audio, video]).await?; // Ctrl-C will be caught in `save`
        resource.on_status(StatusFlags::SAVED);
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct QrInfo {
    url: String,
    qrcode_key: String,
}

#[derive(Debug, serde::Deserialize)]
struct Wbi {
    img_url: String,
    sub_url: String,
}

#[derive(Debug, serde::Deserialize)]
struct Dash {
    #[serde(deserialize_with = "extract")]
    audio: Url,
    #[serde(deserialize_with = "extract")]
    video: Url,
}

/// Extract the url from the json value
fn extract<'de, D>(d: D) -> Result<Url, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: serde_json::Value = serde::Deserialize::deserialize(d)?;
    let url = v[0]["base_url"].as_str().unwrap().to_string();
    Ok(Url::parse(&url).unwrap())
}

fn try_extract_cookie(resp: &Response) -> FavUtilsResult<HashMap<String, String>> {
    let cookies = resp.cookies().collect::<Vec<_>>();
    if cookies.is_empty() {
        return Err(FavUtilsError::NoCookie);
    }
    Ok(cookies
        .iter()
        .map(|c| (c.name().to_string(), c.value().to_string()))
        .collect())
}

#[cfg(test)]
mod tests {
    use fav_core::ops::OperationsExt;

    use super::*;
    use crate::proto::bili::BiliSets;

    #[tokio::test]
    #[ignore = "need to scan qr code manually"]
    async fn login_test() {
        let mut bili = Bili::default();
        bili.login().await.unwrap();
    }

    #[tokio::test]
    async fn ops_test() {
        let bili = Bili::read().unwrap();
        let mut sets = BiliSets::default();
        bili.fetch_sets(&mut sets).await.unwrap();
        let set = sets.iter_mut().min_by_key(|s| s.media_count).unwrap();
        bili.fetch_set(set).await.unwrap();
        bili.fetch_all(set).await.unwrap();
        bili.pull_all(set).await.unwrap();
    }
}
