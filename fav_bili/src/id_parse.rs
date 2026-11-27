use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};
use url::Url;

const XOR_CODE: u64 = 23_442_827_791_579;
const MASK_CODE: u64 = (1u64 << 51) - 1;
const MAX_AID: u64 = 1u64 << 51;
const BASE: u64 = 58;
const ALPHABET: &[u8; 58] = b"FcwAPNKTMug3GV5Lj7EJnHpWsx4tb8haYeviqBz6rkCy12mUSDQX9RdoZf";

/// Typed Bilibili media identifier parsed from CLI input (AID, BVID, or video URL).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaIdent {
    Aid(i64),
    Bvid(String),
}

impl MediaIdent {
    /// Parse a single user-supplied video identifier (`BV1...`, `av123`, `123`, or bilibili URL).
    pub fn parse(input: &str) -> Result<Self> {
        parse_media_ident(input)
    }

    /// Resolve to a numeric AID (computing offline if this is a `Bvid`).
    pub fn to_aid(&self) -> Option<i64> {
        match self {
            Self::Aid(aid) => Some(*aid),
            Self::Bvid(bvid) => bvid_to_aid(bvid),
        }
    }

    /// Resolve to a canonical `BV1...` BVID (computing offline if this is an `Aid`).
    pub fn to_bvid(&self) -> Option<String> {
        match self {
            Self::Aid(aid) => aid_to_bvid(*aid),
            Self::Bvid(bvid) => Some(bvid.clone()),
        }
    }

    /// Return both `(aid, bvid)` if conversion succeeds.
    pub fn as_pair(&self) -> Option<(i64, String)> {
        Some((self.to_aid()?, self.to_bvid()?))
    }
}

impl fmt::Display for MediaIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aid(aid) => write!(f, "av{aid}"),
            Self::Bvid(bvid) => f.write_str(bvid),
        }
    }
}

impl FromStr for MediaIdent {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_media_ident(s)
    }
}

fn alphabet_index(b: u8) -> Option<u64> {
    ALPHABET.iter().position(|&c| c == b).map(|idx| idx as u64)
}

/// Convert a Bilibili AID (`1 <= aid < 2^51`) into its canonical 12-char `BV1...` string.
pub fn aid_to_bvid(aid: i64) -> Option<String> {
    if aid <= 0 {
        return None;
    }
    let aid_u64 = aid as u64;
    if aid_u64 >= MAX_AID {
        return None;
    }

    let mut bytes = *b"BV1000000000";
    let mut bv_idx = 11usize;
    let mut tmp = (MAX_AID | aid_u64) ^ XOR_CODE;
    while tmp != 0 {
        if bv_idx < 3 {
            return None;
        }
        bytes[bv_idx] = ALPHABET[(tmp % BASE) as usize];
        tmp /= BASE;
        bv_idx -= 1;
    }
    bytes.swap(3, 9);
    bytes.swap(4, 7);
    std::str::from_utf8(&bytes).ok().map(str::to_owned)
}

/// Convert a 12-character Bilibili BVID (`BV1...`) into its numeric AID offline.
pub fn bvid_to_aid(bvid: &str) -> Option<i64> {
    let trimmed = bvid.trim();
    if trimmed.len() != 12 || !trimmed.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 12];
    bytes.copy_from_slice(trimmed.as_bytes());
    if !bytes[0].eq_ignore_ascii_case(&b'B')
        || !bytes[1].eq_ignore_ascii_case(&b'V')
        || bytes[2] != b'1'
    {
        return None;
    }
    bytes[0] = b'B';
    bytes[1] = b'V';
    let normalized = std::str::from_utf8(&bytes).ok()?.to_owned();

    bytes.swap(3, 9);
    bytes.swap(4, 7);

    let mut tmp: u64 = 0;
    for &b in &bytes[3..] {
        let digit = alphabet_index(b)?;
        tmp = tmp.checked_mul(BASE)?.checked_add(digit)?;
    }

    if tmp < MAX_AID || (tmp & !MASK_CODE) != MAX_AID {
        return None;
    }
    let aid_u64 = (tmp & MASK_CODE) ^ XOR_CODE;
    if aid_u64 == 0 || aid_u64 >= MAX_AID {
        return None;
    }
    let aid = i64::try_from(aid_u64).ok()?;
    // Enforce exact bijection so malformed base58 strings are rejected.
    if aid_to_bvid(aid).as_deref() != Some(normalized.as_str()) {
        return None;
    }
    Some(aid)
}

fn normalize_bvid(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    bvid_to_aid(trimmed)?;
    let mut bytes = trimmed.as_bytes().to_vec();
    bytes[0] = b'B';
    bytes[1] = b'V';
    String::from_utf8(bytes).ok()
}

fn parse_positive_aid(raw: &str) -> Option<i64> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let aid = raw.parse::<i64>().ok()?;
    if aid > 0 && (aid as u64) < MAX_AID {
        Some(aid)
    } else {
        None
    }
}

fn parse_token(token: &str) -> Option<MediaIdent> {
    let t = token.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(norm) = normalize_bvid(t) {
        return Some(MediaIdent::Bvid(norm));
    }
    if t.len() > 2 && t[..2].eq_ignore_ascii_case("av") {
        return parse_positive_aid(&t[2..]).map(MediaIdent::Aid);
    }
    parse_positive_aid(t).map(MediaIdent::Aid)
}

fn is_bilibili_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "bilibili.com" || lower.ends_with(".bilibili.com")
}

/// Parsed video target with an optional 1-based page number extracted from `?p=N` in URLs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedMediaTarget {
    pub ident: MediaIdent,
    pub page: Option<i64>,
}

/// Parse a BVID (`BV1...`), `av<id>`, raw numeric `<id>`, or Bilibili video URL,
/// additionally extracting a positive `?p=N` page query parameter if present.
pub fn parse_media_target(input: &str) -> Result<ParsedMediaTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("empty media identifier");
    }

    // Fast path: direct BVID / avID / numeric AID
    if let Some(ident) = parse_token(trimmed) {
        return Ok(ParsedMediaTarget { ident, page: None });
    }

    // URL path (with or without explicit scheme)
    let maybe_url = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Url::parse(trimmed).ok()
    } else if trimmed.contains('/') && trimmed.contains('.') {
        Url::parse(&format!("https://{trimmed}")).ok()
    } else {
        None
    };

    if let Some(url) = maybe_url {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if host == "b23.tv" || host.ends_with(".b23.tv") {
            bail!(
                "short link '{trimmed}' cannot be resolved offline; pass a full bilibili.com/video/BV... URL or BV/av ID"
            );
        }
        if !is_bilibili_host(&host) {
            bail!("unsupported URL host '{host}' in '{trimmed}' (expected bilibili.com)");
        }
        let page = url.query_pairs().find_map(|(k, v)| {
            if k == "p" {
                v.trim().parse::<i64>().ok().filter(|&p| p >= 1)
            } else {
                None
            }
        });
        if let Some(segments) = url.path_segments() {
            for seg in segments {
                let seg = seg.trim();
                if seg.is_empty() {
                    continue;
                }
                if let Some(norm) = normalize_bvid(seg) {
                    return Ok(ParsedMediaTarget {
                        ident: MediaIdent::Bvid(norm),
                        page,
                    });
                }
                if seg.len() > 2
                    && seg[..2].eq_ignore_ascii_case("av")
                    && let Some(aid) = parse_positive_aid(&seg[2..])
                {
                    return Ok(ParsedMediaTarget {
                        ident: MediaIdent::Aid(aid),
                        page,
                    });
                }
            }
        }
        return Err(anyhow!(
            "no valid BV or av video identifier found in URL '{trimmed}'"
        ));
    }

    Err(anyhow!(
        "invalid media identifier '{trimmed}' (expected BV1..., av<id>, numeric aid, or bilibili.com/video/... URL)"
    ))
}

/// Parse a BVID (`BV1...`), `av<id>`, raw numeric `<id>`, or Bilibili video URL.
pub fn parse_media_ident(input: &str) -> Result<MediaIdent> {
    parse_media_target(input).map(|t| t.ident)
}

/// Parse multiple media identifiers, returning the first error encountered.
pub fn parse_media_idents(inputs: &[String]) -> Result<Vec<MediaIdent>> {
    inputs.iter().map(|s| parse_media_ident(s)).collect()
}

const MAX_PAGE_RANGE_SPAN: i64 = 10_000;

fn parse_positive_page(raw: &str, full_spec: &str) -> Result<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        bail!("invalid page number '{trimmed}' in page spec '{full_spec}'");
    }
    let page = trimmed
        .parse::<i64>()
        .map_err(|_| anyhow!("invalid page number '{trimmed}' in page spec '{full_spec}'"))?;
    if page < 1 {
        bail!("page number must be >= 1 in page spec '{full_spec}' (got {page})");
    }
    Ok(page)
}

/// Parse a 1-based page selection spec such as `"1"`, `"2-4"`, or `"1,3,5-7"`.
pub fn parse_page_spec(spec: &str) -> Result<BTreeSet<i64>> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        bail!("page spec must not be empty");
    }
    let mut pages = BTreeSet::new();
    for part in trimmed.split(',') {
        let token = part.trim();
        if token.is_empty() {
            bail!("empty entry in page spec '{spec}'");
        }
        if let Some((start_raw, end_raw)) = token.split_once('-') {
            let start = parse_positive_page(start_raw, spec)?;
            let end = parse_positive_page(end_raw, spec)?;
            if start > end {
                bail!("inverted page range '{token}' in page spec '{spec}' (start > end)");
            }
            if end - start > MAX_PAGE_RANGE_SPAN {
                bail!(
                    "page range '{token}' exceeds maximum span of {MAX_PAGE_RANGE_SPAN} in page spec '{spec}'"
                );
            }
            for p in start..=end {
                pages.insert(p);
            }
        } else {
            let p = parse_positive_page(token, spec)?;
            pages.insert(p);
        }
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_bvid_aid_conversions() {
        let cases = [
            ("BV17x411w7KC", 170_001_i64),
            ("BV1Q541167Qg", 455_017_605_i64),
            ("BV1xx411c7mD", 2_i64),
        ];
        for (bvid, aid) in cases {
            assert_eq!(bvid_to_aid(bvid), Some(aid), "bvid_to_aid({bvid})");
            assert_eq!(
                aid_to_bvid(aid).as_deref(),
                Some(bvid),
                "aid_to_bvid({aid})"
            );
        }
    }

    #[test]
    fn roundtrip_various_aids() {
        for aid in [1, 7, 100, 170_001, 455_017_605, 1_130_000_000_000_000] {
            let bvid = aid_to_bvid(aid).expect("valid aid should convert");
            assert!(bvid.starts_with("BV1"));
            assert_eq!(bvid.len(), 12);
            assert_eq!(bvid_to_aid(&bvid), Some(aid));
        }
        assert_eq!(aid_to_bvid(0), None);
        assert_eq!(aid_to_bvid(-42), None);
        assert_eq!(aid_to_bvid(MAX_AID as i64), None);
    }

    #[test]
    fn parses_direct_bvid_aid_and_urls() {
        let bv = parse_media_ident("BV17x411w7KC").unwrap();
        assert_eq!(bv, MediaIdent::Bvid("BV17x411w7KC".into()));
        assert_eq!(bv.to_aid(), Some(170_001));

        // Lowercase bv prefix normalized to BV
        let bv_lower = parse_media_ident("bv17x411w7KC").unwrap();
        assert_eq!(bv_lower, MediaIdent::Bvid("BV17x411w7KC".into()));

        let av = parse_media_ident("av455017605").unwrap();
        assert_eq!(av, MediaIdent::Aid(455_017_605));
        assert_eq!(av.to_bvid().as_deref(), Some("BV1Q541167Qg"));

        let av_upper = parse_media_ident("AV170001").unwrap();
        assert_eq!(av_upper, MediaIdent::Aid(170_001));

        let raw = parse_media_ident("170001").unwrap();
        assert_eq!(raw, MediaIdent::Aid(170_001));

        let url_bv = parse_media_ident(
            "https://www.bilibili.com/video/BV1Q541167Qg/?spm_id_from=333.999&p=2#reply123",
        )
        .unwrap();
        assert_eq!(url_bv, MediaIdent::Bvid("BV1Q541167Qg".into()));

        let url_av =
            parse_media_ident("http://m.bilibili.com/video/av170001?p=1&vd_source=abc").unwrap();
        assert_eq!(url_av, MediaIdent::Aid(170_001));

        let schemeless = parse_media_ident("bilibili.com/video/BV17x411w7KC/").unwrap();
        assert_eq!(schemeless, MediaIdent::Bvid("BV17x411w7KC".into()));
    }

    #[test]
    fn rejects_invalid_inputs() {
        for bad in [
            "",
            "   ",
            "0",
            "-123",
            "av0",
            "av-5",
            "BV1short",
            "BV1OOOOOOOOO", // 'O' is not in base58 alphabet
            "BV27x411w7KC", // must start with BV1
            "https://example.com/video/BV17x411w7KC",
            "https://b23.tv/abc123",
            "https://www.bilibili.com/video/",
            "not-a-video",
        ] {
            assert!(
                parse_media_ident(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn parse_media_target_extracts_positive_page_query_param() {
        let bare = parse_media_target("BV1Q541167Qg").unwrap();
        assert_eq!(
            bare,
            ParsedMediaTarget {
                ident: MediaIdent::Bvid("BV1Q541167Qg".into()),
                page: None,
            }
        );

        let with_p = parse_media_target(
            "https://www.bilibili.com/video/BV1Q541167Qg/?spm_id_from=333.999&p=3#reply",
        )
        .unwrap();
        assert_eq!(
            with_p,
            ParsedMediaTarget {
                ident: MediaIdent::Bvid("BV1Q541167Qg".into()),
                page: Some(3),
            }
        );

        let av_with_p =
            parse_media_target("http://m.bilibili.com/video/av170001?p=1&vd_source=abc").unwrap();
        assert_eq!(
            av_with_p,
            ParsedMediaTarget {
                ident: MediaIdent::Aid(170_001),
                page: Some(1),
            }
        );

        let schemeless_p = parse_media_target("bilibili.com/video/BV17x411w7KC?p=12").unwrap();
        assert_eq!(
            schemeless_p,
            ParsedMediaTarget {
                ident: MediaIdent::Bvid("BV17x411w7KC".into()),
                page: Some(12),
            }
        );

        // Non-positive or non-numeric ?p= is ignored (page: None)
        let zero_p =
            parse_media_target("https://www.bilibili.com/video/BV1Q541167Qg/?p=0").unwrap();
        assert_eq!(zero_p.page, None);
        let neg_p =
            parse_media_target("https://www.bilibili.com/video/BV1Q541167Qg/?p=-2").unwrap();
        assert_eq!(neg_p.page, None);
        let bad_p =
            parse_media_target("https://www.bilibili.com/video/BV1Q541167Qg/?p=abc").unwrap();
        assert_eq!(bad_p.page, None);
    }

    #[test]
    fn parse_page_spec_handles_single_ranges_lists_and_rejects_invalid() {
        assert_eq!(parse_page_spec("1").unwrap(), BTreeSet::from([1]));
        assert_eq!(parse_page_spec("2-4").unwrap(), BTreeSet::from([2, 3, 4]));
        assert_eq!(parse_page_spec("3-3").unwrap(), BTreeSet::from([3]));
        assert_eq!(
            parse_page_spec("1,3,5-7").unwrap(),
            BTreeSet::from([1, 3, 5, 6, 7])
        );
        assert_eq!(
            parse_page_spec(" 5 - 7 , 1 , 3 , 6 ").unwrap(),
            BTreeSet::from([1, 3, 5, 6, 7])
        );

        for bad in [
            "", "   ", "0", "-1", "-2-4", "0-3", "5-3", "4-2", "1,,2", ",1", "1,", "1-", "-3",
            "1-2-3", "abc", "1,abc", "1-100000",
        ] {
            assert!(
                parse_page_spec(bad).is_err(),
                "expected page spec '{bad}' to be rejected"
            );
        }
    }
}
