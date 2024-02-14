use crate::proto::bili::Bili;
use fav_core::api::{Api, ApiProvider};

impl ApiProvider<ApiKind> for Bili {
    fn api(&self, api_kind: ApiKind) -> &dyn Api {
        match api_kind {
            ApiKind::Qr => &QrApi,
            ApiKind::QrPoll => &QrPollApi,
            ApiKind::Logout => &LogoutApi,
        }
    }
}

/// The kinds of bilibili APIs, provided for `ApiProvider`
pub(super) enum ApiKind {
    Qr,
    QrPoll,
    Logout,
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
#[api(endpoint("https://passport.bilibili.com/login/exit/v2"), params(&["biliCSRF"]))]
struct LogoutApi;
