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

impl Operations<ApiKind> for Bili {
    async fn login(&mut self) -> FavCoreResult<()> {
        let resp = self.request(ApiKind::Qr, []).await?;
        let QrInfo { url, qrcode_key } = resp2serde(resp, "/data").await?;
        show_qr_code(url)?;
        // Expired after 120s
        for _ in 0..40 {
            sleep(Duration::from_secs(POLL_INTERVAL)).await;
            let resp = self.request(ApiKind::QrPoll, [qrcode_key.as_str()]).await?;
            if let Ok(cookies) = try_extract_cookie(&resp) {
                self.set_cookies(cookies);
                return Ok(());
            }
        }
        Err(FavCoreError::UtilsError(Box::new(FavUtilsError::QrExpired)))
    }

    async fn logout(&mut self) -> FavCoreResult<()> {
        todo!()
    }

    async fn fetch(&self, resource: &mut impl Meta) -> FavCoreResult<()> {
        todo!()
    }

    async fn pull(&self, resource: &mut impl Meta) -> FavCoreResult<()> {
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
    async fn test_login() {
        let mut bili = Bili::default();
        bili.login().await.unwrap();
    }
}
