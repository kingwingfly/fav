use anyhow::anyhow;
use api_req::{ApiCaller, RedirectPolicy, header};

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15";
const REFERER: &str = "https://www.bilibili.com/";

#[derive(Debug, ApiCaller)]
#[api_req(
    base_url = "https://passport.bilibili.com",
    default_headers = (
        (header::USER_AGENT, USER_AGENT),
        (header::REFERER, REFERER),
    ),
    redirect = RedirectPolicy::custom(|attempt| {
        let url = attempt.url().to_string();
        attempt.error(anyhow!("Bili risk control redirectes to {}", url))
    })
)]
pub struct AuthApi;

#[derive(Debug, ApiCaller)]
#[api_req(
    base_url = "https://api.bilibili.com",
    default_headers = (
        (header::USER_AGENT, USER_AGENT),
        (header::REFERER, REFERER),
    ),
    redirect = RedirectPolicy::custom(|attempt| {
        let url = attempt.url().to_string();
        attempt.error(anyhow!("Bili risk control redirectes to {}", url))
    })
)]
pub struct BiliApi;
