use std::collections::HashMap;

use super::api::ApiKind;
use crate::{
    proto::bili::Bili,
    utils::{parse::resp2serde, qr::show_qr_code},
    FavUtilsError, FavUtilsResult,
};
use fav_core::prelude::*;
use reqwest::Response;
use tokio::time::{sleep, Duration};

const POLL_INTERVAL: u64 = 3;
#[cfg(not(test))]
const EXPIRED_DURATION: u64 = 120;
#[cfg(test)]
const EXPIRED_DURATION: u64 = 3;
const HINT: &str = "Never Login";

impl Operations<ApiKind> for Bili {
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

    async fn fetch(&self, resource: &mut impl Meta) -> FavCoreResult<()> {
        let params = &[self.cookies().get("DedeUserID").expect(HINT).as_str()];
        let resp = self.request(ApiKind::FetchFavSets, params).await?;
        let json: serde_json::Value = resp.json().await.unwrap();
        dbg!(json);
        resource.on_status(StatusFlags::FETCHED);
        Ok(())
    }

    async fn pull(&self, _resource: &mut impl Meta) -> FavCoreResult<()> {
        todo!()
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

    #[tokio::test]
    #[should_panic(expected = "Expired")]
    async fn login_test() {
        let mut bili = Bili::default();
        bili.login().await.unwrap();
    }

    #[tokio::test]
    async fn fetch_test() {
        let bili = Bili::read();
        let mut res = crate::proto::bili::Res::default();
        bili.fetch(&mut res).await.unwrap();
    }
}
