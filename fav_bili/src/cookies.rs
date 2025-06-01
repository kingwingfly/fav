use anyhow::{Context as _, Result};
use api_req::{COOKIE_JAR, CookieStore as _};
use cookie::Cookie;

pub fn parse_cookies(cookies: &str) -> impl Iterator<Item = Cookie<'_>> {
    Cookie::split_parse_encoded(cookies).filter_map(|res| res.ok())
}

/// Set `api_req::COOKIE_JAR` with cookies of account_id from db.
pub fn add_cookie_jar<'a>(cookies: impl Iterator<Item = Cookie<'a>>) {
    cookies.into_iter().for_each(|mut c| {
        c.set_domain("bilibili.com");
        COOKIE_JAR.add_cookie_str(
            &c.encoded().to_string(),
            &"https://bilibili.com".parse().unwrap(),
        );
    });
}

pub fn current_cookies() -> Result<String> {
    Ok(COOKIE_JAR
        .cookies(&"https://bilibili.com".parse().unwrap())
        .context("Auth related cookies should be set.")?
        .to_str()?
        .to_owned())
}
