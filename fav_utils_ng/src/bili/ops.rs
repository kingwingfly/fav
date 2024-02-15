use super::api::ApiKind;
use crate::{
    proto::bili::{Bili, BiliRes, BiliSet, BiliSets},
    utils::{
        parse::{resp2proto, resp2serde},
        qr::show_qr_code,
    },
    FavUtilsError, FavUtilsResult,
};
use fav_core::prelude::*;
use reqwest::Response;
use std::collections::HashMap;
use tokio::time::{sleep, Duration};

const POLL_INTERVAL: u64 = 3;
#[cfg(not(test))]
const EXPIRED_DURATION: u64 = 120;
#[cfg(test)]
const EXPIRED_DURATION: u64 = 3;
const HINT: &str = "Never Login";

impl Operations<BiliSets, BiliSet, BiliRes, ApiKind> for Bili {
    async fn login(&mut self) -> FavCoreResult<()> {
        let resp = self.request(ApiKind::Qr, &[]).await?;
        let QrInfo { url, qrcode_key } = resp2serde(resp, "/data").await?;
        show_qr_code(url)?;
        // Expired after 120s
        for _ in 0..EXPIRED_DURATION / POLL_INTERVAL {
            sleep(Duration::from_secs(POLL_INTERVAL)).await;
            let resp = self
                .request(ApiKind::QrPoll, &[qrcode_key.as_str()])
                .await?;
            if let Ok(cookies) = try_extract_cookie(&resp) {
                self.extend_cookies(cookies);
                return Ok(());
            }
        }
        Err(FavCoreError::UtilsError(Box::new(FavUtilsError::QrExpired)))
    }

    async fn logout(&mut self) -> FavCoreResult<()> {
        let params = &[self.cookies().get("bili_jct").expect(HINT).as_str()];
        let resp = self.request(ApiKind::Logout, params).await?;
        match resp2serde::<i32>(resp, "/code").await? {
            0 => Ok(()),
            _ => Err(FavCoreError::UtilsError(Box::new(
                FavUtilsError::LogoutError,
            ))),
        }
    }

    async fn fetch_sets(&self) -> FavCoreResult<BiliSets> {
        let params = &[self.cookies().get("DedeUserID").expect(HINT).as_str()];
        let resp = self.request(ApiKind::FetchSets, params).await?;
        resp2proto::<BiliSets>(resp, "/data").await
    }

    async fn fetch_set(&self, set: &mut BiliSet) -> FavCoreResult<()> {
        let id = set.id.to_string();

        for pn in 1..=set.media_count.saturating_sub(1) / 20 + 1 {
            let pn = pn.to_string();
            let params = &[id.as_str(), pn.as_str(), "20"];
            let resp = self.request(ApiKind::FetchSet, params).await?;
            let res: BiliSet = resp2proto(resp, "/data").await?;
            res.medias.into_iter().for_each(|mut r| {
                if !set.medias.iter().any(|r1| r1.bvid == r.bvid) {
                    r.on_status(StatusFlags::FAV);
                    set.medias.push(r);
                }
            });
        }
        Ok(())
    }

    async fn fetch(&self, _resource: &mut BiliRes) -> FavCoreResult<()> {
        Ok(())
    }

    async fn pull(&self, _resource: &mut BiliRes) -> FavCoreResult<()> {
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct QrInfo {
    url: String,
    qrcode_key: String,
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
    use super::*;
    use crate::proto::bili::BiliSets;

    #[tokio::test]
    #[should_panic(expected = "Expired")]
    async fn login_test() {
        let mut bili = Bili::default();
        bili.login().await.unwrap();
    }

    #[tokio::test]
    async fn fetch_test() {
        let bili = Bili::read();
        let mut sets: BiliSets = bili.fetch_sets().await.unwrap();
        let set = sets.iter_mut().next().unwrap();
        bili.fetch_set(set).await.unwrap();
        dbg!(set);
    }
}
