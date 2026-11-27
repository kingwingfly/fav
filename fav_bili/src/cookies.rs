use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use reqwest::cookie::{CookieStore as _, Jar};
use url::Url;

pub const BILIBILI_ORIGIN: &str = "https://bilibili.com";

/// Parse a `Cookie` header string (e.g. `SESSDATA=abc; bili_jct=def`) into an ordered map.
///
/// Tolerates an optional leading `Cookie:` prefix, whitespace, trailing semicolons, and `=` inside
/// cookie values (splitting only on the first `=`). Later duplicates overwrite earlier ones by
/// cookie name.
pub fn parse_cookie_header(raw: &str) -> BTreeMap<String, String> {
    let trimmed = raw.trim();
    let body = trimmed
        .strip_prefix("Cookie:")
        .or_else(|| trimmed.strip_prefix("cookie:"))
        .unwrap_or(trimmed);

    let mut map = BTreeMap::new();
    for segment in body.split(';') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((name, value)) = segment.split_once('=') {
            let name = name.trim();
            let value = value.trim();
            if !name.is_empty() {
                map.insert(name.to_string(), value.to_string());
            }
        }
    }
    map
}

/// Parse a Netscape HTTP Cookie File (`cookies.txt`) extracting unexpired cookies scoped to
/// `bilibili.com`.
///
/// Supports `#HttpOnly_` prefixed rows used by browser cookie exporters and `yt-dlp`.
pub fn parse_netscape_cookies(content: &str) -> BTreeMap<String, String> {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    parse_netscape_cookies_at(content, now_unix)
}

/// Parse a Netscape HTTP Cookie File (`cookies.txt`) relative to `now_unix` (Unix epoch seconds),
/// skipping rows whose non-zero expiration timestamp is at or before `now_unix`.
///
/// Session cookies (`expiration == 0`) and cookies with future expiration timestamps are retained.
pub fn parse_netscape_cookies_at(content: &str, now_unix: u64) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let effective = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            rest
        } else if line.starts_with('#') {
            continue;
        } else {
            line
        };

        let fields: Vec<&str> = effective.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }
        let domain = fields[0]
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if domain != "bilibili.com" && !domain.ends_with(".bilibili.com") {
            continue;
        }
        if let Ok(exp) = fields[4].trim().parse::<i64>()
            && (exp < 0 || (exp > 0 && (exp as u64) <= now_unix))
        {
            continue;
        }
        let name = fields[5].trim();
        let value = fields[6].trim();
        if !name.is_empty() {
            map.insert(name.to_string(), value.to_string());
        }
    }
    map
}

/// Merge `updates` into `base`, overwriting existing keys by cookie name.
pub fn merge_cookies(base: &mut BTreeMap<String, String>, updates: &BTreeMap<String, String>) {
    for (k, v) in updates {
        if !k.is_empty() {
            base.insert(k.clone(), v.clone());
        }
    }
}

/// Format an ordered cookie map as a `Cookie` header string (`name=value; name2=value2`).
pub fn format_cookie_header(cookies: &BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Verify that `SESSDATA` is present and non-empty.
pub fn validate_sessdata(cookies: &BTreeMap<String, String>) -> Result<()> {
    match cookies.get("SESSDATA") {
        Some(v) if !v.trim().is_empty() => Ok(()),
        _ => Err(anyhow!(
            "Cookies must include a non-empty `SESSDATA` value."
        )),
    }
}

/// Extract Bilibili's CSRF token (`bili_jct`) if present and non-empty.
pub fn extract_csrf(cookies: &BTreeMap<String, String>) -> Option<String> {
    cookies
        .get("bili_jct")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

/// Build an isolated `reqwest::cookie::Jar` seeded for `bilibili.com` subdomains.
pub fn build_jar_for_account(cookies: &str) -> Arc<Jar> {
    let url = Url::parse(BILIBILI_ORIGIN).expect("valid static bilibili origin URL");
    build_jar_for_url(cookies, &url)
}

/// Build an isolated `reqwest::cookie::Jar` seeded for `url` (using `Domain=bilibili.com` when
/// targeting bilibili.com, or host-only cookies for local test servers like `127.0.0.1`).
pub fn build_jar_for_url(cookies: &str, url: &Url) -> Arc<Jar> {
    let jar = Arc::new(Jar::default());
    let parsed = parse_cookie_header(cookies);
    for (name, value) in parsed {
        set_cookie_in_jar(&jar, url, &name, &value);
    }
    jar
}

/// Insert or replace a single cookie by name inside `jar` for `url`.
pub fn set_cookie_in_jar(jar: &Jar, url: &Url, name: &str, value: &str) {
    let is_bilibili = url
        .host_str()
        .is_some_and(|h| h == "bilibili.com" || h.ends_with(".bilibili.com"));
    let cookie_str = if is_bilibili {
        format!("{name}={value}; Domain=bilibili.com; Path=/")
    } else {
        format!("{name}={value}; Path=/")
    };
    jar.add_cookie_str(&cookie_str, url);
}

/// Snapshot all cookies that `jar` would send to `url` as a sorted `BTreeMap`.
pub fn jar_to_cookie_map(jar: &Jar, url: &Url) -> BTreeMap<String, String> {
    jar.cookies(url)
        .and_then(|hv| hv.to_str().ok().map(parse_cookie_header))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cookie_header_handles_equals_whitespace_and_prefix() {
        let raw = "Cookie: SESSDATA=abc%2C123==; bili_jct=csrf_tok ; ; buvid3=xyz=1; ";
        let map = parse_cookie_header(raw);
        assert_eq!(map.get("SESSDATA").map(String::as_str), Some("abc%2C123=="));
        assert_eq!(map.get("bili_jct").map(String::as_str), Some("csrf_tok"));
        assert_eq!(map.get("buvid3").map(String::as_str), Some("xyz=1"));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn parse_netscape_cookies_filters_domain_and_handles_httponly() {
        let txt = "\
# Netscape HTTP Cookie File
# This is a comment

.example.com\tTRUE\t/\tFALSE\t0\tother\tignored
#HttpOnly_.bilibili.com\tTRUE\t/\tTRUE\t1999999999\tSESSDATA\tsecret_val==
.bilibili.com\tTRUE\t/\tFALSE\t1999999999\tbili_jct\tcsrf123
api.bilibili.com\tFALSE\t/\tFALSE\t0\tbuvid3\tBUV-1
";
        let map = parse_netscape_cookies(txt);
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("SESSDATA").map(String::as_str),
            Some("secret_val==")
        );
        assert_eq!(map.get("bili_jct").map(String::as_str), Some("csrf123"));
        assert_eq!(map.get("buvid3").map(String::as_str), Some("BUV-1"));
        assert!(!map.contains_key("other"));
    }

    #[test]
    fn parse_netscape_cookies_at_skips_expired_and_keeps_future_and_session() {
        let now = 1_700_000_000u64;
        let txt = "\
# Netscape HTTP Cookie File
.bilibili.com\tTRUE\t/\tTRUE\t1699999999\tSESSDATA\texpired_sess
.bilibili.com\tTRUE\t/\tTRUE\t1700000000\tbili_jct\texact_now_expired
.bilibili.com\tTRUE\t/\tTRUE\t-1\tneg_cookie\tnegative_expiry_expired
#HttpOnly_.bilibili.com\tTRUE\t/\tTRUE\t1700000001\tDedeUserID\tfuture_uid
.bilibili.com\tTRUE\t/\tFALSE\t0\tbuvid3\tsession_buvid
.bilibili.com\tTRUE\t/\tTRUE\t1600000000\tDedeUserID\texpired_duplicate_must_not_overwrite
";
        let map = parse_netscape_cookies_at(txt, now);
        assert!(!map.contains_key("SESSDATA"));
        assert!(!map.contains_key("bili_jct"));
        assert!(!map.contains_key("neg_cookie"));
        assert_eq!(
            map.get("DedeUserID").map(String::as_str),
            Some("future_uid")
        );
        assert_eq!(map.get("buvid3").map(String::as_str), Some("session_buvid"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn merge_cookies_overwrites_by_name_without_duplicates() {
        let mut base = parse_cookie_header("SESSDATA=old; bili_ticket=expired_tok; buvid3=keep");
        let updates = parse_cookie_header("bili_ticket=fresh_tok; bili_ticket_expires=999999");
        merge_cookies(&mut base, &updates);
        assert_eq!(base.get("SESSDATA").map(String::as_str), Some("old"));
        assert_eq!(
            base.get("bili_ticket").map(String::as_str),
            Some("fresh_tok")
        );
        assert_eq!(
            base.get("bili_ticket_expires").map(String::as_str),
            Some("999999")
        );
        assert_eq!(base.get("buvid3").map(String::as_str), Some("keep"));
        assert_eq!(base.len(), 4);
    }

    #[test]
    fn jars_are_isolated_per_account() {
        let jar_a = build_jar_for_account("SESSDATA=aaa; bili_jct=csrf_a; extra_a=1");
        let jar_b = build_jar_for_account("SESSDATA=bbb");

        let api_url = Url::parse("https://api.bilibili.com/x/web-interface/nav").unwrap();
        let map_a = jar_to_cookie_map(&jar_a, &api_url);
        let map_b = jar_to_cookie_map(&jar_b, &api_url);

        assert_eq!(map_a.get("SESSDATA").map(String::as_str), Some("aaa"));
        assert_eq!(map_a.get("bili_jct").map(String::as_str), Some("csrf_a"));
        assert_eq!(map_b.get("SESSDATA").map(String::as_str), Some("bbb"));
        assert!(
            !map_b.contains_key("bili_jct"),
            "account A cookies must never leak into account B"
        );
        assert!(!map_b.contains_key("extra_a"));
    }

    #[test]
    fn validate_sessdata_and_extract_csrf() {
        let empty = BTreeMap::new();
        assert!(validate_sessdata(&empty).is_err());
        assert_eq!(extract_csrf(&empty), None);

        let valid = parse_cookie_header("SESSDATA=tok; bili_jct=abc");
        assert!(validate_sessdata(&valid).is_ok());
        assert_eq!(extract_csrf(&valid), Some("abc".to_string()));

        let formatted = format_cookie_header(&valid);
        assert_eq!(formatted, "SESSDATA=tok; bili_jct=abc");
    }
}
