use crate::proto::bili::Bili;
use fav_core::api::{Api, ApiProvider};

impl ApiProvider<ApiKind> for Bili {
    fn api(&self, api_kind: ApiKind) -> &dyn Api {
        match api_kind {
            ApiKind::Qr => &QrApi,
            ApiKind::QrPoll => &QrPollApi,
            ApiKind::Logout => &LogoutApi,
            ApiKind::FetchSets => &SetsApi,
            ApiKind::FetchSet => &SetApi,
        }
    }
}

/// The kinds of bilibili APIs, provided for `ApiProvider`
pub(super) enum ApiKind {
    Qr,
    QrPoll,
    Logout,
    FetchSets,
    FetchSet,
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
