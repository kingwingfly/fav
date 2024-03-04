use super::Bili;
use crate::utils::time::timestamp_sc;
use fav_core::api::{Api, ApiProvider};

impl ApiProvider for Bili {
    type ApiKind = ApiKind;

    fn api(&self, api_kind: ApiKind) -> &dyn Api {
        match api_kind {
            ApiKind::Qr => &QrApi,
            ApiKind::QrPoll => &QrPollApi,
            ApiKind::Logout => &LogoutApi,
            ApiKind::FetchSets => &SetsApi,
            ApiKind::FetchSet => &SetApi,
            ApiKind::FetchRes => &ResApi,
            ApiKind::Wbi => &WbiApi,
            ApiKind::Pull => &PullApi,
        }
    }
}

/// The kinds of bilibili APIs, provided for `ApiProvider`
pub enum ApiKind {
    Qr,
    QrPoll,
    Logout,
    FetchSets,
    FetchSet,
    FetchRes,
    Wbi,
    Pull,
}

/// The bilibili QR code generation API
#[derive(Api)]
#[api(endpoint("https://passport.bilibili.com/x/passport-login/web/qrcode/generate"))]
struct QrApi;

/// The bilibili QR code result polling API
#[derive(Api)]
#[api(endpoint("https://passport.bilibili.com/x/passport-login/web/qrcode/poll"), params(&["qrcode_key"]))]
struct QrPollApi;

#[derive(Api)]
#[api(endpoint("https://passport.bilibili.com/login/exit/v2"), params(&["biliCSRF"]), cookies(&["DedeUserID", "bili_jct", "SESSDATA"]), method(POST))]
struct LogoutApi;

#[derive(Api)]
#[api(endpoint("https://api.bilibili.com/x/v3/fav/folder/created/list-all"), params(&["up_mid"]), cookies(&["SESSDATA"]))]
struct SetsApi;

#[derive(Api)]
#[api(endpoint("https://api.bilibili.com/x/v3/fav/resource/list"), params(&["media_id", "pn", "ps"]), cookies(&["SESSDATA"]))]
struct SetApi;

#[derive(Api)]
#[api(endpoint("https://api.bilibili.com/x/web-interface/view"), params(&["bvid"]), cookies(&["SESSDATA"]))]
struct ResApi;

#[derive(Api)]
#[api(endpoint("https://api.bilibili.com/x/web-interface/nav"))]
struct WbiApi;

const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

struct PullApi;

impl Api for PullApi {
    fn endpoint(&self) -> &'static str {
        "https://api.bilibili.com/x/player/wbi/playurl"
    }

    fn params(&self) -> &[&str] {
        &["bvid", "cid", "qn", "fnval", "fourk", "sub_key", "img_key"]
    }

    fn cookie_keys(&self) -> &[&str] {
        &["SESSDATA"]
    }

    fn url(&self, params: Vec<String>) -> reqwest::Url {
        let mut params: Vec<_> = self.params().iter().copied().zip(params).collect();
        let key = format!("{}{}", params.pop().unwrap().1, params.pop().unwrap().1);
        let url = format!("{}?{}", self.endpoint(), encode_wbi(params, key));
        reqwest::Url::parse(&url).unwrap()
    }
}

fn encode_wbi(mut params: Vec<(&str, String)>, key: String) -> String {
    let mixin_key = get_mixin_key(key.as_bytes());
    let cur_time = timestamp_sc();
    params.push(("wts", cur_time.to_string()));
    params.sort_by(|a, b| a.0.cmp(b.0));
    let query = params.iter().fold(String::from(""), |acc, (k, v)| {
        acc + format!("{}={}&", get_url_encoded(k), get_url_encoded(v)).as_str()
    });
    let web_sign = format!("{:?}", md5::compute(query.clone() + &mixin_key));
    query + &format!("w_rid={}", web_sign)
}

fn get_mixin_key(orig: &[u8]) -> String {
    MIXIN_KEY_ENC_TAB
        .iter()
        .map(|&i| orig[i] as char)
        .collect::<String>()
}

fn get_url_encoded(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c.is_ascii_alphanumeric() || "-_.~".contains(c) {
            true => Some(c.to_string()),
            false => {
                // 过滤 value 中的 "!'()*" 字符
                if "!'()*".contains(c) {
                    return None;
                }
                let encoded = c
                    .encode_utf8(&mut [0; 4])
                    .bytes()
                    .fold("".to_string(), |acc, b| acc + &format!("%{:02X}", b));
                Some(encoded)
            }
        })
        .collect::<String>()
}
