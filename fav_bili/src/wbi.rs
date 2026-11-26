use std::{
    collections::BTreeMap,
    future::Future,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::sync::RwLock;
use url::Url;

const MIXIN_KEY_ENC_TAB: [u8; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

/// RFC-3986 unreserved set matching Bilibili's WBI spec (`A-Z`, `a-z`, `0-9`, `-`, `_`, `.`, `~`).
/// Encodes space as `%20` (never `+`) and uses uppercase hex.
const WBI_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

const DEFAULT_WBI_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Extract the WBI key stem from a Bilibili `img_url` or `sub_url`.
pub fn extract_key_from_url(url: &Url) -> Result<String> {
    let segment = url
        .path_segments()
        .and_then(|mut segs| segs.next_back())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("WBI URL '{url}' missing filename path segment"))?;
    let stem = segment
        .split('.')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("WBI URL '{url}' has empty key stem"))?;
    Ok(stem.to_string())
}

/// Derive Bilibili's 32-char WBI mixin key from `img_key` and `sub_key` with full bounds checking.
pub fn mixin_key(img_key: &str, sub_key: &str) -> Result<String> {
    let combined = format!("{img_key}{sub_key}");
    if combined.len() < 64 {
        return Err(anyhow!(
            "Invalid WBI key length: expected at least 64 bytes, got {}",
            combined.len()
        ));
    }
    if !combined.is_ascii() {
        return Err(anyhow!("Invalid WBI key: expected ASCII characters"));
    }
    let bytes = combined.as_bytes();
    let mut out = String::with_capacity(32);
    for &idx in MIXIN_KEY_ENC_TAB.iter().take(32) {
        let b = *bytes
            .get(idx as usize)
            .ok_or_else(|| anyhow!("WBI mixin index {idx} out of bounds"))?;
        out.push(b as char);
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WbiSigned {
    pub query: String,
    pub w_rid: String,
    pub wts: u64,
}

pub struct WbiEncoder;

impl WbiEncoder {
    /// Strip Bilibili WBI disallowed characters `!'()*` from a parameter value.
    pub fn filter_value(val: &str) -> String {
        val.chars()
            .filter(|c| !matches!(c, '!' | '\'' | '(' | ')' | '*'))
            .collect()
    }

    /// Percent-encode a single WBI key or filtered value according to RFC-3986 (`%20` for spaces).
    pub fn encode_component(val: &str) -> String {
        utf8_percent_encode(val, WBI_ENCODE_SET).to_string()
    }

    /// Sort parameters lexicographically, append `wts`, strip `!'()*`, RFC-3986 encode, and sign
    /// with MD5(`query + mixin_key`). Returns the full signed query string (`...&w_rid=...`).
    pub fn sign(params: &BTreeMap<String, String>, mixin: &str, wts: u64) -> Result<WbiSigned> {
        if mixin.len() < 32 {
            return Err(anyhow!(
                "Invalid WBI mixin key length: expected 32, got {}",
                mixin.len()
            ));
        }

        let mut sorted = params.clone();
        sorted.insert("wts".to_string(), wts.to_string());

        let base_query = sorted
            .iter()
            .map(|(k, v)| {
                let filtered = Self::filter_value(v);
                format!(
                    "{}={}",
                    Self::encode_component(k),
                    Self::encode_component(&filtered)
                )
            })
            .collect::<Vec<_>>()
            .join("&");

        let mut digest_input = String::with_capacity(base_query.len() + mixin.len());
        digest_input.push_str(&base_query);
        digest_input.push_str(mixin);
        let w_rid = format!("{:x}", md5::compute(digest_input));

        let query = format!("{base_query}&w_rid={w_rid}");
        Ok(WbiSigned { query, w_rid, wts })
    }

    /// Convenience wrapper returning only the full signed query string.
    pub fn encode_query(
        params: &BTreeMap<String, String>,
        mixin: &str,
        wts: u64,
    ) -> Result<String> {
        Ok(Self::sign(params, mixin, wts)?.query)
    }
}

#[derive(Debug, Clone)]
struct CachedWbi {
    mixin_key: String,
    fetched_at: Instant,
}

#[derive(Debug)]
pub struct WbiCache {
    inner: RwLock<Option<CachedWbi>>,
    ttl: Duration,
}

impl Default for WbiCache {
    fn default() -> Self {
        Self::new(DEFAULT_WBI_TTL)
    }
}

impl WbiCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: RwLock::new(None),
            ttl,
        }
    }

    pub async fn get(&self) -> Option<String> {
        let guard = self.inner.read().await;
        guard
            .as_ref()
            .filter(|entry| entry.fetched_at.elapsed() < self.ttl)
            .map(|entry| entry.mixin_key.clone())
    }

    pub async fn set(&self, mixin_key: String) {
        let mut guard = self.inner.write().await;
        *guard = Some(CachedWbi {
            mixin_key,
            fetched_at: Instant::now(),
        });
    }

    pub async fn invalidate(&self) {
        let mut guard = self.inner.write().await;
        *guard = None;
    }

    /// Return a cached mixin key if fresh, otherwise run `fetch` once under write lock and cache it.
    pub async fn get_or_fetch<F, Fut>(&self, fetch: F) -> Result<String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String>>,
    {
        if let Some(key) = self.get().await {
            return Ok(key);
        }
        let mut guard = self.inner.write().await;
        if let Some(entry) = guard.as_ref()
            && entry.fetched_at.elapsed() < self.ttl
        {
            return Ok(entry.mixin_key.clone());
        }
        let fresh = fetch().await?;
        *guard = Some(CachedWbi {
            mixin_key: fresh.clone(),
            fetched_at: Instant::now(),
        });
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn mixin_key_matches_bilibili_reference_vector() {
        let img_key = "7cd084941338484aae1ad9425b84077c";
        let sub_key = "4932caff0ff746eab6f01bf08b70ac45";
        let key = mixin_key(img_key, sub_key).unwrap();
        assert_eq!(key, "ea1db124af3c7062474693fa704f4ff8");
    }

    #[test]
    fn wbi_sign_matches_reference_vector_and_sorts_keys() {
        let mixin = mixin_key(
            "7cd084941338484aae1ad9425b84077c",
            "4932caff0ff746eab6f01bf08b70ac45",
        )
        .unwrap();
        let mut params = BTreeMap::new();
        params.insert("foo".to_string(), "114".to_string());
        params.insert("bar".to_string(), "514".to_string());
        params.insert("baz".to_string(), "1919810".to_string());

        let signed = WbiEncoder::sign(&params, &mixin, 1_702_204_169).unwrap();
        let expected_rid = format!(
            "{:x}",
            md5::compute(format!("bar=514&baz=1919810&foo=114&wts=1702204169{mixin}"))
        );
        assert_eq!(signed.w_rid, expected_rid);
        assert_eq!(
            signed.query,
            format!("bar=514&baz=1919810&foo=114&wts=1702204169&w_rid={expected_rid}")
        );
    }

    #[test]
    fn wbi_strips_special_chars_and_uses_percent20_for_spaces() {
        let mixin = "ea1db124af3c7062474693fa704f4ff8";
        let mut params = BTreeMap::new();
        params.insert("q".to_string(), "hello world!'()*中文".to_string());

        let signed = WbiEncoder::sign(&params, mixin, 1_700_000_000).unwrap();
        assert!(
            signed
                .query
                .starts_with("q=hello%20world%E4%B8%AD%E6%96%87&wts=1700000000&w_rid="),
            "unexpected signed query: {}",
            signed.query
        );
        assert!(!signed.query.contains('+'), "spaces must be %20, not +");
    }

    #[test]
    fn short_or_invalid_wbi_keys_return_err_without_panicking() {
        assert!(mixin_key("short", "key").is_err());
        assert!(mixin_key("", "").is_err());

        let bad_url = Url::parse("https://i0.hdslb.com/").unwrap();
        assert!(extract_key_from_url(&bad_url).is_err());

        let good_url =
            Url::parse("https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png")
                .unwrap();
        assert_eq!(
            extract_key_from_url(&good_url).unwrap(),
            "7cd084941338484aae1ad9425b84077c"
        );
    }

    #[tokio::test]
    async fn wbi_cache_caches_and_invalidates() {
        let cache = WbiCache::new(Duration::from_secs(60));
        let calls = AtomicUsize::new(0);

        let k1 = cache
            .get_or_fetch(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("ea1db124af3c7062474693fa704f4ff8".to_string())
            })
            .await
            .unwrap();
        let k2 = cache
            .get_or_fetch(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("should_not_be_called".to_string())
            })
            .await
            .unwrap();

        assert_eq!(k1, k2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        cache.invalidate().await;
        let k3 = cache
            .get_or_fetch(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("0123456789abcdef0123456789abcdef".to_string())
            })
            .await
            .unwrap();
        assert_eq!(k3, "0123456789abcdef0123456789abcdef");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
