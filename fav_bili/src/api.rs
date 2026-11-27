use std::{
    collections::BTreeMap,
    fmt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow};
use reqwest::{
    Client, StatusCode,
    cookie::Jar,
    header::{CONTENT_RANGE, HeaderMap, HeaderValue, RANGE, REFERER, USER_AGENT},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    fs::OpenOptions,
    io::{AsyncWriteExt as _, BufWriter},
    sync::Mutex,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use url::Url;

use crate::{
    cookies::{
        BILIBILI_ORIGIN, build_jar_for_url, extract_csrf, format_cookie_header, jar_to_cookie_map,
        set_cookie_in_jar,
    },
    payload::{
        CollectedSetsPayload, ConfirmRefreshPayload, CookieRefreshPayload, DashPayload,
        FollowingNumPayload, FollowingUpPayload, InSetPayload, InUpPayload, LikePayload,
        ListSetPayload, LogoutPayload, MediaInfoPayload, PublishNumPayload, QrPollPayload,
        SeasonArchivesPayload, TicketPayload,
    },
    response::{
        AccountInfo, BiliEnvelope, Buvid3Data, CollectedSetsData, CookieInfoData,
        CookieRefreshData, DashData, FollowingNumData, FollowingUpData, InSetData, InUpData,
        ListSetData, LogoutResp, MediaInfoData, PublishNumData, QrData, QrPollData,
        SeasonArchivesData, Set, TicketData,
    },
    wbi::{WbiCache, WbiEncoder},
};

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15";
pub const DEFAULT_REFERER: &str = "https://www.bilibili.com/";
pub const DEFAULT_API_BASE_URL: &str = "https://api.bilibili.com";
pub const DEFAULT_PASSPORT_BASE_URL: &str = "https://passport.bilibili.com";

static JITTER_SEED: AtomicU64 = AtomicU64::new(1);

/// Structured Bilibili JSON API error (`code != 0`).
#[derive(Debug, Clone)]
pub struct BiliApiError {
    pub code: i64,
    pub message: String,
    pub endpoint: String,
}

impl BiliApiError {
    pub fn new(code: i64, message: Option<String>, endpoint: impl Into<String>) -> Self {
        Self {
            code,
            message: message.unwrap_or_default(),
            endpoint: endpoint.into(),
        }
    }

    pub fn is_auth_expired(&self) -> bool {
        self.code == -101
    }

    pub fn is_risk_or_rate_limited(&self) -> bool {
        matches!(self.code, -352 | -412 | -799 | -509)
    }

    pub fn is_server_error(&self) -> bool {
        matches!(self.code, -500 | -502 | -503 | -504)
    }

    pub fn is_retryable(&self) -> bool {
        self.is_risk_or_rate_limited() || self.is_server_error()
    }

    pub fn is_wbi_rejected(&self) -> bool {
        matches!(self.code, -352 | -403)
    }

    pub fn is_media_expired(&self) -> bool {
        matches!(self.code, -404 | 62002 | 62004)
    }

    pub fn is_permission_denied(&self) -> bool {
        matches!(self.code, -403 | 62012 | -10403)
    }
}

impl fmt::Display for BiliApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            write!(f, "Bilibili API {} error code {}", self.endpoint, self.code)
        } else {
            write!(
                f,
                "Bilibili API {} error code {}: {}",
                self.endpoint, self.code, self.message
            )
        }
    }
}

impl std::error::Error for BiliApiError {}

#[derive(Debug, Clone)]
pub struct BiliClientConfig {
    pub api_base_url: Url,
    pub passport_base_url: Url,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub api_request_timeout: Duration,
    pub max_retries: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub min_request_interval: Duration,
    pub wbi_ttl: Duration,
}

impl Default for BiliClientConfig {
    fn default() -> Self {
        Self {
            api_base_url: Url::parse(DEFAULT_API_BASE_URL).expect("valid default API URL"),
            passport_base_url: Url::parse(DEFAULT_PASSPORT_BASE_URL)
                .expect("valid default passport URL"),
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(60),
            api_request_timeout: Duration::from_secs(120),
            max_retries: 4,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(16),
            min_request_interval: Duration::from_millis(120),
            wbi_ttl: Duration::from_secs(6 * 60 * 60),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DownloadOptions {
    /// Maximum sustained download rate in bytes/sec (`None` = unlimited).
    pub limit_rate: Option<u64>,
    /// Optional custom `Referer` override for CDN requests.
    pub referer: Option<String>,
    /// Optional cancellation token checked between CDN chunks.
    pub cancel: Option<CancellationToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub delta: u64,
}

/// Per-account Bilibili HTTP client owning an isolated `reqwest::cookie::Jar`, WBI key cache,
/// retry/backoff policy, and polite request pacing.
#[derive(Clone)]
pub struct BiliClient {
    client: Client,
    jar: Arc<Jar>,
    cookie_url: Url,
    config: BiliClientConfig,
    wbi_cache: Arc<WbiCache>,
    last_request: Arc<Mutex<Option<Instant>>>,
}

impl fmt::Debug for BiliClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BiliClient")
            .field("api_base_url", &self.config.api_base_url)
            .field("passport_base_url", &self.config.passport_base_url)
            .finish()
    }
}

impl BiliClient {
    /// Create a client seeded with an account's cookie string using default Bilibili endpoints.
    pub fn new(cookies: &str) -> Result<Self> {
        Self::with_config(cookies, BiliClientConfig::default())
    }

    /// Create an anonymous client with an empty cookie jar (e.g. for QR code login).
    pub fn anonymous() -> Result<Self> {
        Self::new("")
    }

    /// Create a client with custom endpoints/timeouts (useful for local mock server testing).
    pub fn with_config(cookies: &str, config: BiliClientConfig) -> Result<Self> {
        let cookie_url = if is_bilibili_host(&config.api_base_url) {
            Url::parse(BILIBILI_ORIGIN).expect("valid bilibili origin")
        } else {
            config.api_base_url.clone()
        };
        let jar = build_jar_for_url(cookies, &cookie_url);

        let mut default_headers = HeaderMap::new();
        default_headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
        default_headers.insert(REFERER, HeaderValue::from_static(DEFAULT_REFERER));

        let client = Client::builder()
            .cookie_provider(jar.clone())
            .default_headers(default_headers)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .build()
            .context("Failed to construct reqwest HTTP client")?;

        let wbi_cache = Arc::new(WbiCache::new(config.wbi_ttl));
        Ok(Self {
            client,
            jar,
            cookie_url,
            config,
            wbi_cache,
            last_request: Arc::new(Mutex::new(None)),
        })
    }

    pub fn api_base_url(&self) -> &Url {
        &self.config.api_base_url
    }

    pub fn passport_base_url(&self) -> &Url {
        &self.config.passport_base_url
    }

    pub fn wbi_cache(&self) -> &WbiCache {
        &self.wbi_cache
    }

    /// Export this client's current cookies as a deterministic `Cookie` header string.
    pub fn export_cookies(&self) -> String {
        let map = self.export_cookie_map();
        format_cookie_header(&map)
    }

    /// Export this client's current cookies as a sorted `BTreeMap`.
    pub fn export_cookie_map(&self) -> BTreeMap<String, String> {
        jar_to_cookie_map(&self.jar, &self.cookie_url)
    }

    /// Ensure `bili_ticket` (unexpired) and `buvid3` are present in this client's own jar.
    ///
    /// Returns `Ok(true)` if any cookie was fetched/updated and should be persisted back to DB.
    pub async fn ensure_anti_bot_cookies(&self) -> Result<bool> {
        let cookies = self.export_cookie_map();
        let csrf = extract_csrf(&cookies).unwrap_or_default();
        let now = unix_now_secs();
        let mut updated = false;

        let ticket_valid = cookies.contains_key("bili_ticket")
            && cookies
                .get("bili_ticket_expires")
                .and_then(|v| v.parse::<u64>().ok())
                .is_some_and(|exp| exp > now.saturating_add(60));

        if !ticket_valid {
            debug!("Refreshing bili_ticket cookie");
            let data = self.gen_web_ticket(&csrf).await?;
            let expires = data.created_at.saturating_add(data.ttl).to_string();
            set_cookie_in_jar(&self.jar, &self.cookie_url, "bili_ticket", &data.ticket);
            set_cookie_in_jar(&self.jar, &self.cookie_url, "bili_ticket_expires", &expires);
            updated = true;
        }

        if !cookies.contains_key("buvid3") {
            debug!("Fetching missing buvid3 cookie");
            let buvid = self.get_buvid3().await?;
            set_cookie_in_jar(&self.jar, &self.cookie_url, "buvid3", &buvid);
            updated = true;
        }

        Ok(updated)
    }

    // ---- Authentication & Account APIs ----

    pub async fn qr_generate(&self) -> Result<QrData> {
        self.get_envelope(
            &self.config.passport_base_url,
            "/x/passport-login/web/qrcode/generate",
            &(),
        )
        .await
    }

    pub async fn qr_poll(&self, qrcode_key: &str) -> Result<QrPollData> {
        let payload = QrPollPayload {
            qrcode_key: qrcode_key.to_string(),
        };
        self.get_envelope(
            &self.config.passport_base_url,
            "/x/passport-login/web/qrcode/poll",
            &payload,
        )
        .await
    }

    pub async fn cookie_info(&self) -> Result<CookieInfoData> {
        self.get_envelope(
            &self.config.passport_base_url,
            "/x/passport-login/web/cookie/info",
            &(),
        )
        .await
    }

    pub async fn cookie_refresh(
        &self,
        payload: &CookieRefreshPayload,
    ) -> Result<CookieRefreshData> {
        self.post_form_envelope(
            &self.config.passport_base_url,
            "/x/passport-login/web/cookie/refresh",
            payload,
        )
        .await
    }

    pub async fn confirm_refresh(&self, payload: &ConfirmRefreshPayload) -> Result<()> {
        let url = join_url(
            &self.config.passport_base_url,
            "/x/passport-login/web/confirm/refresh",
        )?;
        let env: BiliEnvelope<serde_json::Value> = self
            .send_json_with_retry("/x/passport-login/web/confirm/refresh", || {
                self.client
                    .post(url.clone())
                    .timeout(self.config.api_request_timeout)
                    .form(payload)
            })
            .await?;
        if env.code != 0 {
            return Err(BiliApiError::new(
                env.code,
                env.message,
                "/x/passport-login/web/confirm/refresh",
            )
            .into());
        }
        Ok(())
    }

    pub async fn logout(&self, bili_csrf: &str) -> Result<()> {
        let url = join_url(&self.config.passport_base_url, "/login/exit/v2")?;
        let payload = LogoutPayload {
            biliCSRF: bili_csrf.to_string(),
        };
        let resp: LogoutResp = self
            .send_json_with_retry("/login/exit/v2", || {
                self.client
                    .post(url.clone())
                    .timeout(self.config.api_request_timeout)
                    .form(&payload)
            })
            .await?;
        if resp.code != 0 {
            return Err(BiliApiError::new(resp.code, resp.message, "/login/exit/v2").into());
        }
        Ok(())
    }

    /// Fetch `/x/web-interface/nav`.
    ///
    /// Note: Bilibili returns `code = -101` with `data.isLogin = false` (and valid `wbi_img`) when
    /// cookies are expired or anonymous; in that case `AccountInfo` is still returned so callers
    /// can inspect `info.is_login` or extract WBI keys cleanly.
    pub async fn nav_info(&self) -> Result<AccountInfo> {
        let path = "/x/web-interface/nav";
        let url = join_url(&self.config.api_base_url, path)?;
        let env: BiliEnvelope<AccountInfo> = self
            .send_json_with_retry(path, || {
                self.client
                    .get(url.clone())
                    .timeout(self.config.api_request_timeout)
            })
            .await?;
        if env.code == 0 || env.code == -101 {
            if let Some(data) = env.data {
                return Ok(data);
            }
            if env.code == -101 {
                return Ok(AccountInfo {
                    is_login: false,
                    ..Default::default()
                });
            }
        }
        Err(BiliApiError::new(env.code, env.message, path).into())
    }

    // ---- Anti-bot Cookie APIs ----

    pub async fn gen_web_ticket(&self, csrf: &str) -> Result<TicketData> {
        let path = "/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket";
        let url = join_url(&self.config.api_base_url, path)?;
        let payload = TicketPayload::new(csrf.to_string());
        let env: BiliEnvelope<TicketData> = self
            .send_json_with_retry(path, || {
                self.client
                    .post(url.clone())
                    .timeout(self.config.api_request_timeout)
                    .query(&payload)
            })
            .await?;
        unwrap_envelope(env, path)
    }

    pub async fn get_buvid3(&self) -> Result<String> {
        let data: Buvid3Data = self
            .get_envelope(&self.config.api_base_url, "/x/web-frontend/getbuvid", &())
            .await?;
        Ok(data.buvid)
    }

    // ---- Favorite Sets / Followings / Media APIs ----

    pub async fn list_created_sets(&self, up_mid: i64) -> Result<Vec<Set>> {
        let path = "/x/v3/fav/folder/created/list-all";
        let url = join_url(&self.config.api_base_url, path)?;
        let payload = ListSetPayload { up_mid };
        for attempt in 0..=self.config.max_retries {
            let env: BiliEnvelope<ListSetData> = self
                .send_json_with_retry(path, || {
                    self.client
                        .get(url.clone())
                        .timeout(self.config.api_request_timeout)
                        .query(&payload)
                })
                .await?;
            if env.code == 0 {
                return Ok(env.data.map(|d| d.list).unwrap_or_default());
            }
            let api_err = BiliApiError::new(env.code, env.message, path);
            if api_err.is_retryable() && attempt < self.config.max_retries {
                warn!(
                    path,
                    code = api_err.code,
                    attempt = attempt + 1,
                    "Transient Bilibili API error; backing off and retrying"
                );
                self.sleep_backoff(attempt).await;
                continue;
            }
            return Err(api_err.into());
        }
        unreachable!("retry loop always returns")
    }

    pub async fn list_collected_sets(
        &self,
        up_mid: i64,
        pn: i64,
        ps: u8,
    ) -> Result<CollectedSetsData> {
        let path = "/x/v3/fav/folder/collected/list";
        let url = join_url(&self.config.api_base_url, path)?;
        let payload = CollectedSetsPayload::new(up_mid, pn, ps);
        for attempt in 0..=self.config.max_retries {
            let env: BiliEnvelope<CollectedSetsData> = self
                .send_json_with_retry(path, || {
                    self.client
                        .get(url.clone())
                        .timeout(self.config.api_request_timeout)
                        .query(&payload)
                })
                .await?;
            if env.code == 0 {
                return Ok(env.data.unwrap_or_default());
            }
            let api_err = BiliApiError::new(env.code, env.message, path);
            if api_err.is_retryable() && attempt < self.config.max_retries {
                warn!(
                    path,
                    code = api_err.code,
                    attempt = attempt + 1,
                    "Transient Bilibili API error; backing off and retrying"
                );
                self.sleep_backoff(attempt).await;
                continue;
            }
            return Err(api_err.into());
        }
        unreachable!("retry loop always returns")
    }

    pub async fn following_count(&self, vmid: i64) -> Result<i64> {
        let data: FollowingNumData = self
            .get_envelope(
                &self.config.api_base_url,
                "/x/relation/stat",
                &FollowingNumPayload { vmid },
            )
            .await?;
        Ok(data.following)
    }

    pub async fn following_ups(&self, vmid: i64, pn: i64, ps: u8) -> Result<FollowingUpData> {
        self.get_envelope(
            &self.config.api_base_url,
            "/x/relation/followings",
            &FollowingUpPayload { vmid, pn, ps },
        )
        .await
    }

    pub async fn set_medias(&self, media_id: i64, pn: i64, ps: u8) -> Result<InSetData> {
        self.get_envelope(
            &self.config.api_base_url,
            "/x/v3/fav/resource/list",
            &InSetPayload::new(media_id, pn, ps),
        )
        .await
    }

    pub async fn season_archives(
        &self,
        mid: i64,
        season_id: i64,
        page_num: i64,
        page_size: u8,
        sort_reverse: bool,
    ) -> Result<SeasonArchivesData> {
        let payload = SeasonArchivesPayload::new(mid, season_id, page_num, page_size, sort_reverse);
        self.get_wbi(
            "/x/polymer/web-space/seasons_archives_list",
            payload.to_wbi_params(),
        )
        .await
    }

    pub async fn publish_count(&self, mid: i64) -> Result<i64> {
        let data: PublishNumData = self
            .get_envelope(
                &self.config.api_base_url,
                "/x/space/navnum",
                &PublishNumPayload { mid },
            )
            .await?;
        Ok(data.video)
    }

    pub async fn up_medias(&self, mid: i64, pn: i64, ps: u8) -> Result<InUpData> {
        let payload = InUpPayload::new(mid, pn, ps);
        self.get_wbi("/x/space/wbi/arc/search", payload.to_wbi_params())
            .await
    }

    pub async fn media_info(&self, aid: i64) -> Result<MediaInfoData> {
        let payload = MediaInfoPayload { aid };
        self.get_wbi("/x/web-interface/wbi/view", payload.to_wbi_params())
            .await
    }

    pub async fn dash_playurl(&self, avid: i64, cid: i64) -> Result<DashData> {
        let payload = DashPayload::new(avid, cid);
        self.dash_playurl_with(&payload).await
    }

    pub async fn dash_playurl_with(&self, payload: &DashPayload) -> Result<DashData> {
        self.get_wbi("/x/player/wbi/playurl", payload.to_wbi_params())
            .await
    }

    pub async fn like_media(&self, aid: i64, like: bool, csrf: &str) -> Result<()> {
        let path = "/x/web-interface/archive/like";
        let url = join_url(&self.config.api_base_url, path)?;
        let payload = if like {
            LikePayload::like(aid, csrf.to_string())
        } else {
            LikePayload::unlike(aid, csrf.to_string())
        };
        let env: BiliEnvelope<serde_json::Value> = self
            .send_json_with_retry(path, || {
                self.client
                    .post(url.clone())
                    .timeout(self.config.api_request_timeout)
                    .form(&payload)
            })
            .await?;
        if env.code != 0 {
            return Err(BiliApiError::new(env.code, env.message, path).into());
        }
        Ok(())
    }

    // ---- Signed WBI Helper ----

    /// Perform a WBI-signed GET request against `api_base_url` with cached WBI keys, automatic
    /// cache invalidation & re-sign on `-352`/`-403`, and retry/backoff on transient errors.
    pub async fn get_wbi<T>(&self, path: &str, params: BTreeMap<String, String>) -> Result<T>
    where
        T: DeserializeOwned,
    {
        for wbi_attempt in 0..2 {
            let mixin = self
                .wbi_cache
                .get_or_fetch(|| async { self.fetch_wbi_mixin_key().await })
                .await?;
            let signed_query = WbiEncoder::encode_query(&params, &mixin, unix_now_secs())?;
            let mut url = join_url(&self.config.api_base_url, path)?;
            url.set_query(Some(&signed_query));

            let res = self
                .send_envelope_with_retry::<T, _>(path, || {
                    self.client
                        .get(url.clone())
                        .timeout(self.config.api_request_timeout)
                })
                .await;

            match res {
                Ok(data) => return Ok(data),
                Err(err) => {
                    let wbi_rejected = err
                        .downcast_ref::<BiliApiError>()
                        .is_some_and(BiliApiError::is_wbi_rejected);
                    if wbi_rejected && wbi_attempt == 0 {
                        debug!(
                            path,
                            "WBI signature rejected; invalidating WBI cache and retrying"
                        );
                        self.wbi_cache.invalidate().await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        unreachable!("wbi_attempt loop always returns within 2 iterations")
    }

    async fn fetch_wbi_mixin_key(&self) -> Result<String> {
        let info = self.nav_info().await?;
        let wbi = info
            .wbi_img
            .ok_or_else(|| anyhow!("Bilibili /x/web-interface/nav response missing wbi_img"))?;
        wbi.mixin_key()
    }

    // ---- Streaming CDN Download Helper ----

    /// Download a media stream into `part_path` with:
    /// - `.error_for_status()` validation
    /// - `Range: bytes=<offset>-` resume when `part_path` already exists
    /// - failover across `urls` (`base_url` then `backup_url` mirrors)
    /// - strict `Content-Length`/`Content-Range` byte-count verification
    /// - buffered async writes via `tokio::fs::File`
    /// - optional sustained rate limiting (`limit_rate` bytes/sec)
    pub async fn download_to_part<F>(
        &self,
        urls: &[Url],
        part_path: &Path,
        opts: &DownloadOptions,
        mut on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(DownloadProgress),
    {
        if urls.is_empty() {
            return Err(anyhow!("No CDN URLs provided for download"));
        }

        if let Some(parent) = part_path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut last_err: Option<anyhow::Error> = None;

        for (url_idx, url) in urls.iter().enumerate() {
            for attempt in 0..=self.config.max_retries {
                if opts
                    .cancel
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled)
                {
                    return Err(anyhow!("Download cancelled"));
                }

                let existing_len = match tokio::fs::metadata(part_path).await {
                    Ok(meta) => meta.len(),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(e) => return Err(e.into()),
                };

                let mut req = self.client.get(url.clone());
                if let Some(ref referer) = opts.referer {
                    req = req.header(REFERER, referer);
                }
                if existing_len > 0 {
                    req = req.header(RANGE, format!("bytes={existing_len}-"));
                }

                let send_res = tokio::select! {
                    biased;
                    _ = wait_cancelled(opts.cancel.as_ref()) => {
                        return Err(anyhow!("Download cancelled"));
                    }
                    res = req.send() => res,
                };
                let resp = match send_res {
                    Ok(r) => r,
                    Err(e) => {
                        last_err = Some(anyhow!("CDN request error ({url}): {e}"));
                        if attempt < self.config.max_retries {
                            self.sleep_backoff_cancellable(attempt, opts.cancel.as_ref())
                                .await?;
                            continue;
                        }
                        break;
                    }
                };

                let status = resp.status();

                // 416 Range Not Satisfiable: either already complete or stale partial length.
                if status == StatusCode::RANGE_NOT_SATISFIABLE && existing_len > 0 {
                    if let Some(total) = parse_content_range_total(resp.headers())
                        && total == existing_len
                    {
                        on_progress(DownloadProgress {
                            downloaded: existing_len,
                            total: Some(total),
                            delta: 0,
                        });
                        return Ok(existing_len);
                    }
                    // Local partial file length exceeds remote; truncate and restart from 0.
                    let _ = tokio::fs::remove_file(part_path).await;
                    continue;
                }

                if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                    last_err = Some(anyhow!("CDN returned HTTP {} for {}", status.as_u16(), url));
                    if attempt < self.config.max_retries {
                        self.sleep_backoff_cancellable(attempt, opts.cancel.as_ref())
                            .await?;
                        continue;
                    }
                    break;
                }

                let mut resp = match resp.error_for_status() {
                    Ok(r) => r,
                    Err(e) => {
                        // 403/404/410 on this CDN node -> fail over immediately to next backup_url.
                        debug!(
                            url_idx,
                            status = status.as_u16(),
                            "CDN node returned client error; trying next mirror if available"
                        );
                        last_err = Some(anyhow!("CDN error ({url}): {e}"));
                        break;
                    }
                };

                let resuming = status == StatusCode::PARTIAL_CONTENT && existing_len > 0;
                let expected_total = if resuming {
                    parse_content_range_total(resp.headers())
                        .or_else(|| resp.content_length().map(|rem| existing_len + rem))
                } else {
                    resp.content_length()
                };

                let file = if resuming {
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(part_path)
                        .await?
                } else {
                    OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(part_path)
                        .await?
                };
                let mut writer = BufWriter::with_capacity(64 * 1024, file);
                let mut downloaded = if resuming { existing_len } else { 0 };
                let session_start = Instant::now();
                let mut session_bytes: u64 = 0;

                on_progress(DownloadProgress {
                    downloaded,
                    total: expected_total,
                    delta: 0,
                });

                let mut stream_err: Option<anyhow::Error> = None;
                loop {
                    let chunk_res = tokio::select! {
                        biased;
                        _ = wait_cancelled(opts.cancel.as_ref()) => {
                            let _ = writer.flush().await;
                            return Err(anyhow!("Download cancelled"));
                        }
                        res = resp.chunk() => res,
                    };
                    match chunk_res {
                        Ok(Some(chunk)) => {
                            writer.write_all(&chunk).await?;
                            let n = chunk.len() as u64;
                            downloaded += n;
                            session_bytes += n;
                            on_progress(DownloadProgress {
                                downloaded,
                                total: expected_total,
                                delta: n,
                            });

                            if let Some(limit_bps) = opts.limit_rate
                                && limit_bps > 0
                            {
                                let expected_elapsed = Duration::from_secs_f64(
                                    session_bytes as f64 / limit_bps as f64,
                                );
                                let actual_elapsed = session_start.elapsed();
                                if expected_elapsed > actual_elapsed {
                                    tokio::select! {
                                        biased;
                                        _ = wait_cancelled(opts.cancel.as_ref()) => {
                                            let _ = writer.flush().await;
                                            return Err(anyhow!("Download cancelled"));
                                        }
                                        _ = tokio::time::sleep(expected_elapsed - actual_elapsed) => {}
                                    }
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            stream_err = Some(anyhow!("Stream read error ({url}): {e}"));
                            break;
                        }
                    }
                }

                writer.flush().await?;

                if let Some(err) = stream_err {
                    last_err = Some(err);
                    if attempt < self.config.max_retries {
                        self.sleep_backoff_cancellable(attempt, opts.cancel.as_ref())
                            .await?;
                        continue;
                    }
                    break;
                }

                if let Some(expected) = expected_total
                    && downloaded != expected
                {
                    last_err = Some(anyhow!(
                        "Incomplete download ({url}): received {downloaded} of {expected} bytes"
                    ));
                    if attempt < self.config.max_retries {
                        self.sleep_backoff_cancellable(attempt, opts.cancel.as_ref())
                            .await?;
                        continue;
                    }
                    break;
                }

                return Ok(downloaded);
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("All CDN download mirrors failed")))
    }

    // ---- Internal HTTP, Retry & Pacing Machinery ----

    async fn get_envelope<P, T>(&self, base: &Url, path: &str, query: &P) -> Result<T>
    where
        P: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let url = join_url(base, path)?;
        self.send_envelope_with_retry(path, || {
            self.client
                .get(url.clone())
                .timeout(self.config.api_request_timeout)
                .query(query)
        })
        .await
    }

    async fn post_form_envelope<P, T>(&self, base: &Url, path: &str, form: &P) -> Result<T>
    where
        P: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let url = join_url(base, path)?;
        self.send_envelope_with_retry(path, || {
            self.client
                .post(url.clone())
                .timeout(self.config.api_request_timeout)
                .form(form)
        })
        .await
    }

    async fn send_envelope_with_retry<T, F>(&self, path: &str, build_req: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn() -> reqwest::RequestBuilder,
    {
        for attempt in 0..=self.config.max_retries {
            let env: BiliEnvelope<T> = self.send_json_with_retry(path, &build_req).await?;
            if env.code == 0 {
                return env
                    .data
                    .ok_or_else(|| anyhow!("Bilibili API {path} returned code=0 with null data"));
            }
            let api_err = BiliApiError::new(env.code, env.message, path);
            if api_err.is_retryable() && attempt < self.config.max_retries {
                warn!(
                    path,
                    code = api_err.code,
                    attempt = attempt + 1,
                    "Transient Bilibili API error; backing off and retrying"
                );
                self.sleep_backoff(attempt).await;
                continue;
            }
            return Err(api_err.into());
        }
        unreachable!("retry loop always returns")
    }

    async fn send_json_with_retry<T, F>(&self, path: &str, build_req: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn() -> reqwest::RequestBuilder,
    {
        for attempt in 0..=self.config.max_retries {
            self.pace().await;

            let resp = match build_req().send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt < self.config.max_retries {
                        warn!(
                            path,
                            attempt = attempt + 1,
                            error = %e,
                            "HTTP transport error; backing off and retrying"
                        );
                        self.sleep_backoff(attempt).await;
                        continue;
                    }
                    return Err(anyhow!("HTTP request to {path} failed: {e}"));
                }
            };

            let status = resp.status();
            if is_retryable_http_status(status) && attempt < self.config.max_retries {
                warn!(
                    path,
                    status = status.as_u16(),
                    attempt = attempt + 1,
                    "Retryable HTTP status; backing off and retrying"
                );
                self.sleep_backoff(attempt).await;
                continue;
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                let snippet: String = body.chars().take(200).collect();
                return Err(anyhow!(
                    "HTTP {} from {}: {}",
                    status.as_u16(),
                    path,
                    snippet.trim()
                ));
            }

            let body = resp
                .text()
                .await
                .with_context(|| format!("Failed reading response body from {path}"))?;
            let parsed = serde_json::from_str::<T>(&body).with_context(|| {
                let snippet: String = body.chars().take(200).collect();
                format!("Failed to deserialize JSON from {path}: {snippet}")
            })?;
            return Ok(parsed);
        }
        unreachable!("retry loop always returns")
    }

    async fn pace(&self) {
        if self.config.min_request_interval.is_zero() {
            return;
        }
        let mut last = self.last_request.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.config.min_request_interval {
                tokio::time::sleep(self.config.min_request_interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    async fn sleep_backoff(&self, attempt: u32) {
        if self.config.base_backoff.is_zero() {
            return;
        }
        let mult = 1u32.checked_shl(attempt.min(10)).unwrap_or(1024);
        let raw = self
            .config
            .base_backoff
            .saturating_mul(mult)
            .min(self.config.max_backoff);
        // Apply jitter in [50%, 100%] without extra crate dependencies.
        let jitter_pct = 50 + (pseudo_rand_u64() % 51);
        let delay = Duration::from_millis((raw.as_millis() as u64 * jitter_pct) / 100);
        tokio::time::sleep(delay).await;
    }

    async fn sleep_backoff_cancellable(
        &self,
        attempt: u32,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        tokio::select! {
            biased;
            _ = wait_cancelled(cancel) => Err(anyhow!("Download cancelled")),
            _ = self.sleep_backoff(attempt) => Ok(()),
        }
    }
}

async fn wait_cancelled(cancel: Option<&CancellationToken>) {
    match cancel {
        Some(token) => token.cancelled().await,
        None => std::future::pending().await,
    }
}

fn unwrap_envelope<T>(env: BiliEnvelope<T>, path: &str) -> Result<T> {
    if env.code != 0 {
        return Err(BiliApiError::new(env.code, env.message, path).into());
    }
    env.data
        .ok_or_else(|| anyhow!("Bilibili API {path} returned code=0 with null data"))
}

fn join_url(base: &Url, path: &str) -> Result<Url> {
    base.join(path)
        .with_context(|| format!("Invalid URL path '{path}' relative to '{base}'"))
}

fn is_bilibili_host(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|h| h == "bilibili.com" || h.ends_with(".bilibili.com"))
}

fn is_retryable_http_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::PRECONDITION_FAILED
        || status.is_server_error()
}

fn parse_content_range_total(headers: &HeaderMap) -> Option<u64> {
    let val = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let (_, total_str) = val.split_once('/')?;
    if total_str == "*" {
        return None;
    }
    total_str.trim().parse().ok()
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn pseudo_rand_u64() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let seq = JITTER_SEED.fetch_add(1, Ordering::Relaxed);
    // SplitMix64 finalizer
    let mut z = nanos.wrapping_add(seq.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tempfile::TempDir;
    use tokio::{io::AsyncReadExt as _, net::TcpListener};

    fn test_config(base: Url) -> BiliClientConfig {
        BiliClientConfig {
            api_base_url: base.clone(),
            passport_base_url: base,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
            api_request_timeout: Duration::from_secs(5),
            max_retries: 3,
            base_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(20),
            min_request_interval: Duration::ZERO,
            wbi_ttl: Duration::from_secs(60),
        }
    }

    async fn spawn_mock_server<F>(handler: F) -> Url
    where
        F: Fn(String, String) -> (u16, Vec<(String, String)>, Vec<u8>) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };
                    let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
                    let first_line = req_str.lines().next().unwrap_or_default();
                    let path = first_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();
                    let (status, headers, body) = handler(path, req_str);
                    let reason = match status {
                        200 => "OK",
                        206 => "Partial Content",
                        403 => "Forbidden",
                        416 => "Range Not Satisfiable",
                        503 => "Service Unavailable",
                        _ => "Status",
                    };
                    let mut resp = format!("HTTP/1.1 {status} {reason}\r\n");
                    let has_cl = headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
                    for (k, v) in headers {
                        resp.push_str(&format!("{k}: {v}\r\n"));
                    }
                    if !has_cl {
                        resp.push_str(&format!("Content-Length: {}\r\n", body.len()));
                    }
                    resp.push_str("Connection: close\r\n\r\n");
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        Url::parse(&format!("http://{addr}")).unwrap()
    }

    #[tokio::test]
    async fn wbi_caching_and_invalidation_on_352() {
        let nav_calls = Arc::new(AtomicUsize::new(0));
        let view_calls = Arc::new(AtomicUsize::new(0));
        let nav_c = nav_calls.clone();
        let view_c = view_calls.clone();

        let base = spawn_mock_server(move |path, _req| {
            if path.starts_with("/x/web-interface/nav") {
                nav_c.fetch_add(1, Ordering::SeqCst);
                let body = r#"{
                    "code": 0,
                    "data": {
                        "isLogin": true,
                        "mid": 42,
                        "uname": "tester",
                        "wbi_img": {
                            "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
                            "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"
                        }
                    }
                }"#;
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    body.as_bytes().to_vec(),
                )
            } else if path.starts_with("/x/web-interface/wbi/view") {
                assert!(path.contains("w_rid="), "view request must be WBI-signed");
                assert!(path.contains("wts="), "view request must include wts");
                let n = view_c.fetch_add(1, Ordering::SeqCst);
                // First attempt returns -352 so WBI cache invalidates & refetches.
                if n == 0 {
                    let body = r#"{"code": -352, "message": "risk", "data": null}"#;
                    (
                        200,
                        vec![("Content-Type".into(), "application/json".into())],
                        body.as_bytes().to_vec(),
                    )
                } else {
                    let body = r#"{
                        "code": 0,
                        "data": {
                            "aid": 170001,
                            "bvid": "BV17x411w7KC",
                            "title": "Demo",
                            "owner": {"mid": 1, "name": "UP"},
                            "pages": [{"cid": 999, "page": 1, "part": "P1"}]
                        }
                    }"#;
                    (
                        200,
                        vec![("Content-Type".into(), "application/json".into())],
                        body.as_bytes().to_vec(),
                    )
                }
            } else {
                (404, vec![], b"{}".to_vec())
            }
        })
        .await;

        let mut cfg = test_config(base);
        cfg.max_retries = 0; // force outer get_wbi -352 invalidation path immediately
        let client = BiliClient::with_config("SESSDATA=ok", cfg).unwrap();

        let info1 = client.media_info(170001).await.unwrap();
        assert_eq!(info1.title, "Demo");
        // Initial nav fetch + 1 refetch after -352 invalidation = 2 nav calls.
        assert_eq!(nav_calls.load(Ordering::SeqCst), 2);

        // Second call uses cached WBI key -> still 2 nav calls total.
        let info2 = client.media_info(170001).await.unwrap();
        assert_eq!(info2.pages[0].cid, 999);
        assert_eq!(nav_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retries_on_503_and_bilibili_799() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let att_c = attempts.clone();

        let base = spawn_mock_server(move |_path, _req| {
            let n = att_c.fetch_add(1, Ordering::SeqCst);
            match n {
                0 => (503, vec![], b"busy".to_vec()),
                1 => (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    br#"{"code": -799, "message": "too fast", "data": null}"#.to_vec(),
                ),
                _ => (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    br#"{"code": 0, "data": {"following": 12}}"#.to_vec(),
                ),
            }
        })
        .await;

        let client = BiliClient::with_config("SESSDATA=ok", test_config(base)).unwrap();
        let count = client.following_count(100).await.unwrap();
        assert_eq!(count, 12);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn ensure_anti_bot_cookies_replaces_expired_ticket_by_name_and_sends_account_cookie() {
        let base = spawn_mock_server(move |path, req| {
            assert!(
                req.contains("SESSDATA=acct_tok"),
                "account cookie must already be loaded before anti-bot requests: {req}"
            );
            if path.starts_with("/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket") {
                let body = r#"{
                    "code": 0,
                    "data": {
                        "ticket": "fresh_ticket_val",
                        "created_at": 4000000000,
                        "ttl": 259200
                    }
                }"#;
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    body.as_bytes().to_vec(),
                )
            } else if path.starts_with("/x/web-frontend/getbuvid") {
                let body = r#"{"code": 0, "data": {"buvid": "BUVID3_FRESH"}}"#;
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    body.as_bytes().to_vec(),
                )
            } else {
                (404, vec![], b"{}".to_vec())
            }
        })
        .await;

        let client = BiliClient::with_config(
            "SESSDATA=acct_tok; bili_jct=csrf1; bili_ticket=old_expired; bili_ticket_expires=1",
            test_config(base),
        )
        .unwrap();

        let updated = client.ensure_anti_bot_cookies().await.unwrap();
        assert!(updated);

        let cookies = client.export_cookie_map();
        assert_eq!(
            cookies.get("bili_ticket").map(String::as_str),
            Some("fresh_ticket_val")
        );
        assert_eq!(
            cookies.get("bili_ticket_expires").map(String::as_str),
            Some("4000259200")
        );
        assert_eq!(
            cookies.get("buvid3").map(String::as_str),
            Some("BUVID3_FRESH")
        );

        // Second call should be a no-op since ticket is now unexpired and buvid3 exists.
        assert!(!client.ensure_anti_bot_cookies().await.unwrap());
    }

    #[tokio::test]
    async fn download_to_part_resumes_range_and_fails_over_backup_url() {
        let full_payload = b"0123456789abcdefghijklmnopqrstuvwxyz".to_vec();
        let payload_clone = full_payload.clone();

        let base = spawn_mock_server(move |path, req| {
            if path == "/primary" {
                return (403, vec![], b"forbidden".to_vec());
            }
            if path == "/backup" {
                if let Some(range_line) = req
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                {
                    let offset: usize = range_line
                        .split("bytes=")
                        .nth(1)
                        .and_then(|s| s.split('-').next())
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(0);
                    let slice = payload_clone[offset..].to_vec();
                    let cr = format!(
                        "bytes {}-{}/{}",
                        offset,
                        payload_clone.len() - 1,
                        payload_clone.len()
                    );
                    return (206, vec![("Content-Range".into(), cr)], slice);
                }
                return (200, vec![], payload_clone.clone());
            }
            (404, vec![], b"".to_vec())
        })
        .await;

        let dir = TempDir::new().unwrap();
        let part = dir.path().join("video.part");
        // Seed first 10 bytes to verify Range resume.
        std::fs::write(&part, &full_payload[..10]).unwrap();

        let client = BiliClient::with_config("", test_config(base.clone())).unwrap();
        let urls = vec![
            base.join("/primary").unwrap(),
            base.join("/backup").unwrap(),
        ];
        let mut last_progress = None;
        let written = client
            .download_to_part(&urls, &part, &DownloadOptions::default(), |p| {
                last_progress = Some(p);
            })
            .await
            .unwrap();

        assert_eq!(written, full_payload.len() as u64);
        assert_eq!(std::fs::read(&part).unwrap(), full_payload);
        assert_eq!(
            last_progress.unwrap().total,
            Some(full_payload.len() as u64)
        );
    }

    #[tokio::test]
    async fn download_to_part_rejects_truncated_transfer_and_retries() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let att_c = attempts.clone();

        let base = spawn_mock_server(move |_path, _req| {
            let n = att_c.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // Claim Content-Length: 20, but only send 5 bytes before closing socket.
                (
                    200,
                    vec![("Content-Length".into(), "20".into())],
                    b"12345".to_vec(),
                )
            } else {
                // Serve remaining 15 bytes on Range resume or full 20 bytes.
                (
                    206,
                    vec![
                        ("Content-Range".into(), "bytes 5-19/20".into()),
                        ("Content-Length".into(), "15".into()),
                    ],
                    b"67890abcdefghij".to_vec(),
                )
            }
        })
        .await;

        let dir = TempDir::new().unwrap();
        let part = dir.path().join("trunc.part");
        let client = BiliClient::with_config("", test_config(base.clone())).unwrap();
        let urls = vec![base.join("/stream").unwrap()];

        let total = client
            .download_to_part(&urls, &part, &DownloadOptions::default(), |_| {})
            .await
            .unwrap();

        assert_eq!(total, 20);
        assert_eq!(std::fs::read(&part).unwrap(), b"1234567890abcdefghij");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_clients_keep_cookies_strictly_isolated() {
        let base = spawn_mock_server(move |_path, req| {
            let body = if req.contains("SESSDATA=acct_one") && !req.contains("SESSDATA=acct_two") {
                r#"{"code": 0, "data": {"following": 11}}"#
            } else if req.contains("SESSDATA=acct_two") && !req.contains("SESSDATA=acct_one") {
                r#"{"code": 0, "data": {"following": 22}}"#
            } else {
                r#"{"code": -101, "message": "cookie leaked", "data": null}"#
            };
            (
                200,
                vec![("Content-Type".into(), "application/json".into())],
                body.as_bytes().to_vec(),
            )
        })
        .await;

        let c1 = BiliClient::with_config("SESSDATA=acct_one; extra=1", test_config(base.clone()))
            .unwrap();
        let c2 = BiliClient::with_config("SESSDATA=acct_two", test_config(base)).unwrap();

        let (r1, r2) = tokio::try_join!(c1.following_count(1), c2.following_count(2)).unwrap();
        assert_eq!(r1, 11);
        assert_eq!(r2, 22);
        assert!(!c2.export_cookie_map().contains_key("extra"));
    }

    #[tokio::test]
    async fn archive_and_set_endpoints_send_expected_query_params() {
        let seen_paths = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_c = seen_paths.clone();

        let base = spawn_mock_server(move |path, _req| {
            seen_c.lock().unwrap().push(path.clone());
            if path.starts_with("/x/web-interface/nav") {
                let body = r#"{
                    "code": 0,
                    "data": {
                        "isLogin": true,
                        "mid": 42,
                        "uname": "tester",
                        "wbi_img": {
                            "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
                            "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"
                        }
                    }
                }"#;
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    body.as_bytes().to_vec(),
                )
            } else if path.starts_with("/x/v3/fav/resource/list") {
                let body = r#"{"code": 0, "data": {"medias": [], "has_more": false}}"#;
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    body.as_bytes().to_vec(),
                )
            } else if path.starts_with("/x/v3/fav/folder/collected/list") {
                let body = r#"{"code": 0, "data": {"count": 1, "has_more": false, "list": [{"id": 500, "mid": 77, "title": "SeasonA", "media_count": 3, "type": 21, "state": 0}]}}"#;
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    body.as_bytes().to_vec(),
                )
            } else if path.starts_with("/x/polymer/web-space/seasons_archives_list") {
                let body = r#"{"code": 0, "data": {"archives": [{"aid": 99, "bvid": "BV1xx411c7mD", "title": "Ep1"}], "page": {"page_num": 1, "page_size": 30, "total": 1}}}"#;
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    body.as_bytes().to_vec(),
                )
            } else {
                (404, vec![], b"{}".to_vec())
            }
        })
        .await;

        let client = BiliClient::with_config("SESSDATA=ok", test_config(base)).unwrap();

        let _ = client.set_medias(123, 1, 20).await.unwrap();
        let collected = client.list_collected_sets(42, 1, 20).await.unwrap();
        assert_eq!(collected.list.len(), 1);
        assert_eq!(collected.list[0].id, 500);

        let archives = client.season_archives(77, 500, 1, 30, true).await.unwrap();
        assert_eq!(archives.archives.len(), 1);
        assert_eq!(archives.archives[0].id, 99);

        let paths = seen_paths.lock().unwrap().clone();
        let set_req = paths
            .iter()
            .find(|p| p.starts_with("/x/v3/fav/resource/list"))
            .unwrap();
        assert!(set_req.contains("order=mtime"), "set_req={set_req}");
        assert!(set_req.contains("platform=web"), "set_req={set_req}");

        let col_req = paths
            .iter()
            .find(|p| p.starts_with("/x/v3/fav/folder/collected/list"))
            .unwrap();
        assert!(col_req.contains("up_mid=42"), "col_req={col_req}");
        assert!(col_req.contains("platform=web"), "col_req={col_req}");

        let season_req = paths
            .iter()
            .find(|p| p.starts_with("/x/polymer/web-space/seasons_archives_list"))
            .unwrap();
        assert!(season_req.contains("mid=77"), "season_req={season_req}");
        assert!(
            season_req.contains("season_id=500"),
            "season_req={season_req}"
        );
        assert!(
            season_req.contains("sort_reverse=true"),
            "season_req={season_req}"
        );
        assert!(
            season_req.contains("web_location=333.999"),
            "season_req={season_req}"
        );
        assert!(season_req.contains("w_rid="), "season_req={season_req}");
    }

    #[tokio::test]
    async fn list_created_sets_retries_transient_799_and_handles_null_data() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let att_c = attempts.clone();

        let base = spawn_mock_server(move |path, _req| {
            assert!(
                path.starts_with("/x/v3/fav/folder/created/list-all"),
                "unexpected path: {path}"
            );
            let n = att_c.fetch_add(1, Ordering::SeqCst);
            let body = match n {
                0 => r#"{"code": -799, "message": "too fast", "data": null}"#,
                1 => {
                    r#"{"code": 0, "data": {"list": [{"id": 100, "title": "Default", "media_count": 5}]}}"#
                }
                _ => r#"{"code": 0, "data": null}"#,
            };
            (
                200,
                vec![("Content-Type".into(), "application/json".into())],
                body.as_bytes().to_vec(),
            )
        })
        .await;

        let client = BiliClient::with_config("SESSDATA=ok", test_config(base)).unwrap();

        let sets = client.list_created_sets(42).await.unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].id, 100);
        assert_eq!(sets[0].title, "Default");
        assert_eq!(sets[0].media_count, 5);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);

        let empty = client.list_created_sets(42).await.unwrap();
        assert!(empty.is_empty());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn download_to_part_cancels_promptly_during_stalled_stream_request_throttle_and_backoff()
    {
        use tokio::sync::Notify;

        let dir = TempDir::new().unwrap();

        // 1. Mid-chunk cancellation on a stalled response stream (flushes partial bytes).
        {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let release = Arc::new(Notify::new());
            let release_srv = release.clone();
            tokio::spawn(async move {
                if let Ok((mut stream, _)) = listener.accept().await {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nhello")
                        .await;
                    let _ = stream.flush().await;
                    release_srv.notified().await;
                }
            });

            let base = Url::parse(&format!("http://{addr}")).unwrap();
            let mut cfg = test_config(base.clone());
            cfg.read_timeout = Duration::from_secs(10);
            let client = BiliClient::with_config("", cfg).unwrap();

            let cancel = CancellationToken::new();
            let got_chunk = Arc::new(Notify::new());
            let got_chunk_cb = got_chunk.clone();
            let cancel_task = cancel.clone();
            tokio::spawn(async move {
                got_chunk.notified().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                cancel_task.cancel();
            });

            let part = dir.path().join("stalled_stream.part");
            let opts = DownloadOptions {
                cancel: Some(cancel),
                ..Default::default()
            };
            let start = Instant::now();
            let err = client
                .download_to_part(&[base.join("/stalled").unwrap()], &part, &opts, |p| {
                    if p.downloaded >= 5 {
                        got_chunk_cb.notify_one();
                    }
                })
                .await
                .unwrap_err();
            release.notify_one();

            assert!(err.to_string().contains("Download cancelled"), "err={err}");
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "mid-chunk cancel took too long: {:?}",
                start.elapsed()
            );
            assert_eq!(std::fs::read(&part).unwrap(), b"hello");
        }

        // 2. Mid-backoff cancellation when server returns retryable 503.
        {
            let base = spawn_mock_server(|_path, _req| (503, vec![], b"busy".to_vec())).await;
            let mut cfg = test_config(base.clone());
            cfg.max_retries = 2;
            cfg.base_backoff = Duration::from_secs(30);
            cfg.max_backoff = Duration::from_secs(30);
            let client = BiliClient::with_config("", cfg).unwrap();

            let cancel = CancellationToken::new();
            let cancel_task = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                cancel_task.cancel();
            });

            let part = dir.path().join("backoff.part");
            let opts = DownloadOptions {
                cancel: Some(cancel),
                ..Default::default()
            };
            let start = Instant::now();
            let err = client
                .download_to_part(&[base.join("/retry").unwrap()], &part, &opts, |_| {})
                .await
                .unwrap_err();
            assert!(err.to_string().contains("Download cancelled"), "err={err}");
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "mid-backoff cancel took too long: {:?}",
                start.elapsed()
            );
        }

        // 3. Mid-throttle cancellation during `--limit-rate` sleep (flushes written bytes).
        {
            let payload = vec![b'x'; 1024];
            let payload_c = payload.clone();
            let base = spawn_mock_server(move |_path, _req| (200, vec![], payload_c.clone())).await;
            let client = BiliClient::with_config("", test_config(base.clone())).unwrap();

            let cancel = CancellationToken::new();
            let got_chunk = Arc::new(Notify::new());
            let got_chunk_cb = got_chunk.clone();
            let cancel_task = cancel.clone();
            tokio::spawn(async move {
                got_chunk.notified().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                cancel_task.cancel();
            });

            let part = dir.path().join("throttle.part");
            let opts = DownloadOptions {
                limit_rate: Some(10),
                cancel: Some(cancel),
                ..Default::default()
            };
            let start = Instant::now();
            let err = client
                .download_to_part(&[base.join("/throttled").unwrap()], &part, &opts, |p| {
                    if p.downloaded > 0 {
                        got_chunk_cb.notify_one();
                    }
                })
                .await
                .unwrap_err();
            assert!(err.to_string().contains("Download cancelled"), "err={err}");
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "mid-throttle cancel took too long: {:?}",
                start.elapsed()
            );
            assert_eq!(std::fs::read(&part).unwrap(), payload);
        }

        // 4. Mid-request cancellation while waiting for HTTP response headers.
        {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let accepted = Arc::new(Notify::new());
            let accepted_srv = accepted.clone();
            let release = Arc::new(Notify::new());
            let release_srv = release.clone();
            tokio::spawn(async move {
                if let Ok((mut stream, _)) = listener.accept().await {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    accepted_srv.notify_one();
                    release_srv.notified().await;
                }
            });

            let base = Url::parse(&format!("http://{addr}")).unwrap();
            let mut cfg = test_config(base.clone());
            cfg.read_timeout = Duration::from_secs(10);
            let client = BiliClient::with_config("", cfg).unwrap();

            let cancel = CancellationToken::new();
            let cancel_task = cancel.clone();
            tokio::spawn(async move {
                accepted.notified().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                cancel_task.cancel();
            });

            let part = dir.path().join("stalled_req.part");
            let opts = DownloadOptions {
                cancel: Some(cancel),
                ..Default::default()
            };
            let start = Instant::now();
            let err = client
                .download_to_part(&[base.join("/hang").unwrap()], &part, &opts, |_| {})
                .await
                .unwrap_err();
            release.notify_one();

            assert!(err.to_string().contains("Download cancelled"), "err={err}");
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "mid-request cancel took too long: {:?}",
                start.elapsed()
            );
        }
    }
}
