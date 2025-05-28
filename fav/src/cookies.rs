use api_req::COOKIE_JAR;
use cookie::Cookie;

pub fn parse_cookies<'a>(cookies: String) -> impl Iterator<Item = Cookie<'a>> {
    Cookie::split_parse_encoded(cookies).filter_map(|res| res.ok())
}

/// Set `api_req::COOKIE_JAR` with cookies of user_id from db.
pub fn set_cookie_jar<'a>(cookies: impl Iterator<Item = Cookie<'a>>) {
    cookies.into_iter().for_each(|mut c| {
        c.set_domain("bilibili.com");
        COOKIE_JAR.add_cookie_str(
            &c.encoded().to_string(),
            &"https://bilibili.com".parse().unwrap(),
        );
    });
}
