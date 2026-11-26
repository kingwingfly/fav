#![allow(dead_code)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};

use fav_bili::api::BiliClientConfig;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use url::Url;

pub const VALID_IMG_URL: &str = "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png";
pub const VALID_SUB_URL: &str = "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png";

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct MockAccount {
    pub mid: i64,
    pub uname: String,
    pub is_login: bool,
    pub nav_code: i64,
    pub sets: Vec<MockSet>,
    pub followings: Vec<MockUp>,
    /// Optional page number on `/x/relation/followings` that should return an API error.
    pub following_error_page: Option<i64>,
}

impl MockAccount {
    pub fn active(mid: i64, uname: impl Into<String>) -> Self {
        Self {
            mid,
            uname: uname.into(),
            is_login: true,
            nav_code: 0,
            sets: Vec::new(),
            followings: Vec::new(),
            following_error_page: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MockSet {
    pub id: i64,
    pub title: String,
    pub media_count: i64,
}

#[derive(Debug, Clone)]
pub struct MockUp {
    pub mid: i64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct MockMediaItem {
    pub id: i64,
    pub bvid: String,
    pub title: String,
    pub media_type: u8,
    pub attr: i64,
    pub upper: Option<MockUp>,
}

#[derive(Debug, Clone)]
pub struct MockPage {
    pub cid: i64,
    pub page: i64,
    pub part: String,
}

#[derive(Debug, Clone)]
pub enum MockViewResponse {
    Ok {
        aid: i64,
        bvid: String,
        title: String,
        owner: MockUp,
        pages: Vec<MockPage>,
    },
    Error {
        code: i64,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct MockStreamSpec {
    pub id: u32,
    pub path: String,
    pub backup_paths: Vec<String>,
    pub mime_type: String,
    pub codecs: String,
    pub bandwidth: u64,
    pub codecid: u32,
}

#[derive(Debug, Clone)]
pub enum MockPlayurlResponse {
    Dash {
        videos: Vec<MockStreamSpec>,
        audios: Option<Vec<MockStreamSpec>>,
    },
    Durl {
        paths: Vec<String>,
    },
    Error {
        code: i64,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct MockCdnAsset {
    pub bytes: Vec<u8>,
    pub status_override: Option<u16>,
    /// If set, truncate the first response (when no Range header) after N bytes while still
    /// advertising the full Content-Length to test truncation detection & Range resume.
    pub truncate_first_unranged: Option<usize>,
    pub hits: usize,
}

impl MockCdnAsset {
    pub fn ok(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            status_override: None,
            truncate_first_unranged: None,
            hits: 0,
        }
    }

    pub fn failing(status: u16) -> Self {
        Self {
            bytes: Vec::new(),
            status_override: Some(status),
            truncate_first_unranged: None,
            hits: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct MockState {
    pub requests: Vec<RecordedRequest>,
    /// Keyed by `SESSDATA` cookie value.
    pub accounts_by_sessdata: HashMap<String, MockAccount>,
    /// Keyed by `set_id`.
    pub set_medias: HashMap<i64, Vec<MockMediaItem>>,
    /// Keyed by `up_id`.
    pub up_medias: HashMap<i64, Vec<MockMediaItem>>,
    /// Keyed by `aid`.
    pub views: HashMap<i64, MockViewResponse>,
    /// Keyed by `(aid, cid)`.
    pub playurls: HashMap<(i64, i64), MockPlayurlResponse>,
    /// Keyed by URL path (e.g. `"/cdn/v1.m4s"`).
    pub cdn_assets: HashMap<String, MockCdnAsset>,
    /// Keyed by `aid` -> `(code, message)`.
    pub like_overrides: HashMap<i64, (i64, String)>,
    /// Keyed by `biliCSRF` -> `code`.
    pub logout_overrides: HashMap<String, i64>,
    pub cookie_needs_refresh: bool,
    pub refresh_new_token: Option<String>,
}

pub struct MockBiliServer {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<MockState>>,
    handle: JoinHandle<()>,
}

impl MockBiliServer {
    pub async fn start(initial_state: MockState) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback mock server");
        let addr = listener.local_addr().expect("mock server addr");
        let state = Arc::new(Mutex::new(initial_state));
        let shared = state.clone();
        let base_origin = format!("http://{addr}");

        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let st = shared.clone();
                let origin = base_origin.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(stream, st, &origin).await;
                });
            }
        });

        Self {
            addr,
            state,
            handle,
        }
    }

    pub fn base_url(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).expect("valid mock URL")
    }

    pub fn client_config(&self) -> BiliClientConfig {
        let url = self.base_url();
        BiliClientConfig {
            api_base_url: url.clone(),
            passport_base_url: url,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
            api_request_timeout: Duration::from_secs(15),
            max_retries: 2,
            base_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(20),
            min_request_interval: Duration::ZERO,
            wbi_ttl: Duration::from_secs(3600),
        }
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn call_count(&self, path_prefix: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.path.starts_with(path_prefix))
            .count()
    }
}

impl Drop for MockBiliServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<MockState>>,
    origin: &str,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_bytes(&buf, b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if buf.len() > 1024 * 1024 {
            return Ok(());
        }
    }

    let head_str = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head_str.lines();
    let req_line = lines.next().unwrap_or("");
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_owned();
    let target = parts.next().unwrap_or("/");
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_owned(), q.to_owned()),
        None => (target.to_owned(), String::new()),
    };

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body =
        String::from_utf8_lossy(&buf[header_end..(header_end + content_length).min(buf.len())])
            .to_string();

    let recorded = RecordedRequest {
        method,
        path: path.clone(),
        query: query.clone(),
        headers: headers.clone(),
        body: body.clone(),
    };

    let (status, extra_headers, advertised_len, payload_bytes) = {
        let mut st = state.lock().unwrap();
        st.requests.push(recorded);
        route_request(&mut st, origin, &path, &query, &headers, &body)
    };

    let status_text = match status {
        200 => "OK",
        206 => "Partial Content",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };

    let mut resp_head = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Length: {advertised_len}\r\nConnection: close\r\n"
    );
    for (k, v) in extra_headers {
        resp_head.push_str(&format!("{k}: {v}\r\n"));
    }
    resp_head.push_str("\r\n");

    stream.write_all(resp_head.as_bytes()).await?;
    stream.write_all(&payload_bytes).await?;
    stream.flush().await?;
    Ok(())
}

fn route_request(
    st: &mut MockState,
    origin: &str,
    path: &str,
    query: &str,
    headers: &HashMap<String, String>,
    body: &str,
) -> (u16, Vec<(String, String)>, usize, Vec<u8>) {
    let q = parse_form(query);
    let form = parse_form(body);
    let cookies = parse_cookie_map(headers.get("cookie").map(String::as_str).unwrap_or(""));
    let sessdata = cookies.get("SESSDATA").cloned().unwrap_or_default();

    if path.starts_with("/cdn/") {
        return serve_cdn(st, path, headers);
    }

    let json_val: Value = match path {
        "/x/web-interface/nav" => {
            if let Some(acc) = st.accounts_by_sessdata.get(&sessdata) {
                if acc.nav_code != 0 {
                    json!({ "code": acc.nav_code, "message": "nav error", "data": null })
                } else {
                    json!({
                        "code": 0,
                        "message": "0",
                        "data": {
                            "isLogin": acc.is_login,
                            "mid": acc.mid,
                            "uname": acc.uname,
                            "wbi_img": {
                                "img_url": VALID_IMG_URL,
                                "sub_url": VALID_SUB_URL,
                            }
                        }
                    })
                }
            } else {
                json!({
                    "code": 0,
                    "message": "0",
                    "data": {
                        "isLogin": false,
                        "wbi_img": {
                            "img_url": VALID_IMG_URL,
                            "sub_url": VALID_SUB_URL,
                        }
                    }
                })
            }
        }
        "/x/web-frontend/getbuvid" | "/x/frontend/finger/spi" => {
            json!({ "code": 0, "message": "0", "data": { "buvid": "MOCK-BUVID3-0001" } })
        }
        "/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket" => {
            json!({
                "code": 0,
                "message": "0",
                "data": {
                    "ticket": format!("ticket_for_{}", cookies.get("bili_jct").map(String::as_str).unwrap_or("anon")),
                    "created_at": 4_102_444_800u64,
                    "ttl": 259200u64,
                }
            })
        }
        "/x/v3/fav/folder/created/list-all" => {
            let up_mid: i64 = q.get("up_mid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let sets: Vec<Value> = st
                .accounts_by_sessdata
                .values()
                .find(|a| a.mid == up_mid)
                .map(|a| {
                    a.sets
                        .iter()
                        .map(|s| {
                            json!({
                                "id": s.id,
                                "title": s.title,
                                "media_count": s.media_count,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({ "code": 0, "message": "0", "data": { "list": sets } })
        }
        "/x/relation/stat" => {
            let vmid: i64 = q.get("vmid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let count = st
                .accounts_by_sessdata
                .values()
                .find(|a| a.mid == vmid)
                .map(|a| a.followings.len() as i64)
                .unwrap_or(0);
            json!({ "code": 0, "message": "0", "data": { "following": count } })
        }
        "/x/relation/followings" => {
            let vmid: i64 = q.get("vmid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let pn: usize = q.get("pn").and_then(|v| v.parse().ok()).unwrap_or(1);
            let ps: usize = q.get("ps").and_then(|v| v.parse().ok()).unwrap_or(50);
            if let Some(acc) = st.accounts_by_sessdata.values().find(|a| a.mid == vmid) {
                if acc.following_error_page == Some(pn as i64) {
                    json!({ "code": -400, "message": "simulated page error", "data": null })
                } else {
                    let start = (pn.saturating_sub(1)) * ps;
                    let slice: Vec<Value> = acc
                        .followings
                        .iter()
                        .skip(start)
                        .take(ps)
                        .map(|u| json!({ "mid": u.mid, "uname": u.name }))
                        .collect();
                    json!({
                        "code": 0,
                        "message": "0",
                        "data": { "list": slice, "total": acc.followings.len() }
                    })
                }
            } else {
                json!({ "code": 0, "message": "0", "data": { "list": [], "total": 0 } })
            }
        }
        "/x/v3/fav/resource/list" => {
            let media_id: i64 = q.get("media_id").and_then(|v| v.parse().ok()).unwrap_or(0);
            let pn: usize = q.get("pn").and_then(|v| v.parse().ok()).unwrap_or(1);
            let ps: usize = q.get("ps").and_then(|v| v.parse().ok()).unwrap_or(20);
            let items = st.set_medias.get(&media_id).cloned().unwrap_or_default();
            let start = (pn.saturating_sub(1)) * ps;
            let has_more = start + ps < items.len();
            let medias: Vec<Value> = items
                .iter()
                .skip(start)
                .take(ps)
                .map(media_item_json)
                .collect();
            json!({
                "code": 0,
                "message": "0",
                "data": { "medias": medias, "has_more": has_more }
            })
        }
        "/x/space/navnum" => {
            let mid: i64 = q.get("mid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let count = st.up_medias.get(&mid).map(|v| v.len() as i64).unwrap_or(0);
            json!({ "code": 0, "message": "0", "data": { "video": count } })
        }
        "/x/space/wbi/arc/search" => {
            let mid: i64 = q.get("mid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let pn: usize = q.get("pn").and_then(|v| v.parse().ok()).unwrap_or(1);
            let ps: usize = q.get("ps").and_then(|v| v.parse().ok()).unwrap_or(30);
            let items = st.up_medias.get(&mid).cloned().unwrap_or_default();
            let start = (pn.saturating_sub(1)) * ps;
            let vlist: Vec<Value> = items
                .iter()
                .skip(start)
                .take(ps)
                .map(media_item_json)
                .collect();
            json!({
                "code": 0,
                "message": "0",
                "data": {
                    "list": { "vlist": vlist },
                    "page": { "pn": pn, "ps": ps, "count": items.len() }
                }
            })
        }
        "/x/web-interface/wbi/view" => {
            let aid: i64 = q.get("aid").and_then(|v| v.parse().ok()).unwrap_or(0);
            match st.views.get(&aid) {
                Some(MockViewResponse::Ok {
                    aid,
                    bvid,
                    title,
                    owner,
                    pages,
                }) => json!({
                    "code": 0,
                    "message": "0",
                    "data": {
                        "aid": aid,
                        "bvid": bvid,
                        "title": title,
                        "owner": { "mid": owner.mid, "name": owner.name },
                        "pages": pages.iter().map(|p| json!({
                            "cid": p.cid,
                            "page": p.page,
                            "part": p.part,
                        })).collect::<Vec<_>>(),
                    }
                }),
                Some(MockViewResponse::Error { code, message }) => {
                    json!({ "code": code, "message": message, "data": null })
                }
                None => json!({ "code": -404, "message": "nothing found", "data": null }),
            }
        }
        "/x/player/wbi/playurl" => {
            let avid: i64 = q.get("avid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let cid: i64 = q.get("cid").and_then(|v| v.parse().ok()).unwrap_or(0);
            match st.playurls.get(&(avid, cid)) {
                Some(MockPlayurlResponse::Dash { videos, audios }) => {
                    let v_json: Vec<Value> =
                        videos.iter().map(|s| stream_spec_json(origin, s)).collect();
                    let a_json = match audios {
                        Some(list) => {
                            Value::Array(list.iter().map(|s| stream_spec_json(origin, s)).collect())
                        }
                        None => Value::Null,
                    };
                    json!({
                        "code": 0,
                        "message": "0",
                        "data": {
                            "quality": 80,
                            "dash": { "video": v_json, "audio": a_json },
                            "durl": null
                        }
                    })
                }
                Some(MockPlayurlResponse::Durl { paths }) => {
                    let durl: Vec<Value> = paths
                        .iter()
                        .map(|p| json!({ "url": format!("{origin}{p}"), "backup_url": [], "size": 0 }))
                        .collect();
                    json!({
                        "code": 0,
                        "message": "0",
                        "data": { "quality": 80, "dash": null, "durl": durl }
                    })
                }
                Some(MockPlayurlResponse::Error { code, message }) => {
                    json!({ "code": code, "message": message, "data": null })
                }
                None => json!({ "code": -404, "message": "no playurl", "data": null }),
            }
        }
        "/x/web-interface/archive/like" => {
            let aid: i64 = form.get("aid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let (code, msg) = st
                .like_overrides
                .get(&aid)
                .cloned()
                .unwrap_or((0, "0".to_owned()));
            json!({ "code": code, "message": msg })
        }
        "/login/exit/v2" => {
            let csrf = form.get("biliCSRF").cloned().unwrap_or_default();
            let expected_csrf = cookies.get("bili_jct").cloned().unwrap_or_default();
            let code = if let Some(&override_code) = st.logout_overrides.get(&csrf) {
                override_code
            } else if !csrf.is_empty() && csrf == expected_csrf {
                0
            } else {
                -111
            };
            let msg = if code == 0 {
                "ok"
            } else {
                "csrf mismatch or invalid session"
            };
            json!({ "code": code, "message": msg })
        }
        "/x/passport-login/web/cookie/info" => {
            json!({
                "code": 0,
                "message": "0",
                "data": { "refresh": st.cookie_needs_refresh, "timestamp": 1700000000i64 }
            })
        }
        "/x/passport-login/web/cookie/refresh" => {
            json!({
                "code": 0,
                "message": "0",
                "data": {
                    "status": 0,
                    "message": "ok",
                    "refresh_token": st.refresh_new_token.clone().unwrap_or_else(|| "refreshed_tok".to_owned())
                }
            })
        }
        "/x/passport-login/web/confirm/refresh" => {
            json!({ "code": 0, "message": "0", "data": null })
        }
        _ => {
            let body = serde_json::to_vec(
                &json!({ "code": -404, "message": format!("unhandled {path}") }),
            )
            .unwrap();
            return (
                404,
                vec![("Content-Type".to_owned(), "application/json".to_owned())],
                body.len(),
                body,
            );
        }
    };

    let bytes = serde_json::to_vec(&json_val).unwrap();
    (
        200,
        vec![("Content-Type".to_owned(), "application/json".to_owned())],
        bytes.len(),
        bytes,
    )
}

fn serve_cdn(
    st: &mut MockState,
    path: &str,
    headers: &HashMap<String, String>,
) -> (u16, Vec<(String, String)>, usize, Vec<u8>) {
    let Some(asset) = st.cdn_assets.get_mut(path) else {
        let msg = b"not found".to_vec();
        return (404, vec![], msg.len(), msg);
    };
    asset.hits += 1;

    if let Some(code) = asset.status_override {
        let msg = format!("status {code}").into_bytes();
        return (code, vec![], msg.len(), msg);
    }

    let total = asset.bytes.len();
    if let Some(range) = headers.get("range")
        && let Some(start_str) = range
            .strip_prefix("bytes=")
            .and_then(|r| r.split('-').next())
        && let Ok(start) = start_str.parse::<usize>()
        && start < total
    {
        let slice = asset.bytes[start..].to_vec();
        let end = total - 1;
        return (
            206,
            vec![
                (
                    "Content-Type".to_owned(),
                    "application/octet-stream".to_owned(),
                ),
                (
                    "Content-Range".to_owned(),
                    format!("bytes {start}-{end}/{total}"),
                ),
            ],
            slice.len(),
            slice,
        );
    }

    if let Some(cut) = asset.truncate_first_unranged.take() {
        let partial = asset.bytes[..cut.min(total)].to_vec();
        return (
            200,
            vec![(
                "Content-Type".to_owned(),
                "application/octet-stream".to_owned(),
            )],
            total, // Advertise full length but only transmit `partial` bytes!
            partial,
        );
    }

    (
        200,
        vec![(
            "Content-Type".to_owned(),
            "application/octet-stream".to_owned(),
        )],
        total,
        asset.bytes.clone(),
    )
}

fn media_item_json(item: &MockMediaItem) -> Value {
    let upper = item
        .upper
        .as_ref()
        .map(|u| json!({ "mid": u.mid, "name": u.name }))
        .unwrap_or(Value::Null);
    json!({
        "id": item.id,
        "bvid": item.bvid,
        "title": item.title,
        "type": item.media_type,
        "attr": item.attr,
        "upper": upper,
    })
}

fn stream_spec_json(origin: &str, s: &MockStreamSpec) -> Value {
    json!({
        "id": s.id,
        "baseUrl": format!("{origin}{}", s.path),
        "backupUrl": s.backup_paths.iter().map(|p| format!("{origin}{p}")).collect::<Vec<_>>(),
        "mimeType": s.mime_type,
        "codecs": s.codecs,
        "bandwidth": s.bandwidth,
        "codecid": s.codecid,
    })
}

fn parse_form(input: &str) -> HashMap<String, String> {
    input
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((k.to_owned(), v.to_owned()))
        })
        .collect()
}

fn parse_cookie_map(header: &str) -> HashMap<String, String> {
    header
        .split(';')
        .filter_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            Some((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Generate tiny valid video-only (`.mp4`) and audio-only (`.m4a`) streams using real `ffmpeg`.
pub fn generate_tiny_av_streams(dir: &Path) -> (PathBuf, PathBuf) {
    let video = dir.join("sample_video.mp4");
    let audio = dir.join("sample_audio.m4a");

    let v_status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=0.4:size=64x64:rate=10",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-an",
        ])
        .arg(&video)
        .status()
        .expect("run ffmpeg to generate test video stream");
    assert!(v_status.success(), "ffmpeg failed generating sample video");

    let a_status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.4",
            "-c:a",
            "aac",
            "-vn",
        ])
        .arg(&audio)
        .status()
        .expect("run ffmpeg to generate test audio stream");
    assert!(a_status.success(), "ffmpeg failed generating sample audio");

    (video, audio)
}
