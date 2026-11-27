mod auth;
mod buvid3;
mod dash;
mod like;
mod media;
mod set;
mod ticket;
mod up;
mod wbi;

use serde::{Deserialize, Deserializer};

pub use auth::*;
pub use buvid3::*;
pub use dash::*;
pub use like::*;
pub use media::*;
pub use set::*;
pub use ticket::*;
pub use up::*;
pub use wbi::*;

/// Standard Bilibili API JSON response envelope.
#[derive(Debug, Clone, Deserialize)]
#[serde(
    bound(deserialize = "T: Deserialize<'de>"),
    from = "RawBiliEnvelope<T>"
)]
pub struct BiliEnvelope<T> {
    pub code: i64,
    pub message: Option<String>,
    pub data: Option<T>,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct RawBiliEnvelope<T> {
    code: i64,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default = "option_none")]
    data: Option<T>,
}

impl<T> From<RawBiliEnvelope<T>> for BiliEnvelope<T> {
    fn from(raw: RawBiliEnvelope<T>) -> Self {
        Self {
            code: raw.code,
            message: pick_optional_str(raw.message, raw.msg),
            data: raw.data,
        }
    }
}

fn option_none<T>() -> Option<T> {
    None
}

/// Prefer the first non-empty string among `primary` and `fallback`, preserving `Some("")`
/// when a key was explicitly present as an empty string and the other was absent or empty.
pub(super) fn pick_optional_str(
    primary: Option<String>,
    fallback: Option<String>,
) -> Option<String> {
    match (primary, fallback) {
        (Some(p), _) if !p.is_empty() => Some(p),
        (_, Some(f)) if !f.is_empty() => Some(f),
        (p, f) => p.or(f),
    }
}

/// Deserialize JSON `null` (or absent with `#[serde(default)]`) as `T::default()`.
pub fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_handles_null_data_and_error_codes() {
        let json = r#"{"code": -352, "message": "risk control", "data": null}"#;
        let env: BiliEnvelope<ListSetData> = serde_json::from_str(json).unwrap();
        assert_eq!(env.code, -352);
        assert_eq!(env.message.as_deref(), Some("risk control"));
        assert!(env.data.is_none());
    }

    #[test]
    fn envelope_handles_dual_message_and_msg_keys() {
        let both = r#"{"code": 0, "message": "0", "msg": "ok", "data": null}"#;
        let env: BiliEnvelope<ListSetData> = serde_json::from_str(both).unwrap();
        assert_eq!(env.code, 0);
        assert_eq!(env.message.as_deref(), Some("0"));

        let empty_message_with_msg =
            r#"{"code": -101, "message": "", "msg": "not logged in", "data": null}"#;
        let env: BiliEnvelope<ListSetData> = serde_json::from_str(empty_message_with_msg).unwrap();
        assert_eq!(env.code, -101);
        assert_eq!(env.message.as_deref(), Some("not logged in"));

        let msg_only = r#"{"code": -400, "msg": "bad request", "data": null}"#;
        let env: BiliEnvelope<ListSetData> = serde_json::from_str(msg_only).unwrap();
        assert_eq!(env.message.as_deref(), Some("bad request"));

        let neither = r#"{"code": 0, "data": null}"#;
        let env: BiliEnvelope<ListSetData> = serde_json::from_str(neither).unwrap();
        assert_eq!(env.message, None);
    }

    #[test]
    fn null_as_default_deserializes_null_arrays_as_empty() {
        let json = r#"{"code": 0, "data": {"list": null}}"#;
        let env: BiliEnvelope<ListSetData> = serde_json::from_str(json).unwrap();
        assert!(env.data.unwrap().list.is_empty());

        let json_medias = r#"{"code": 0, "data": {"medias": null, "has_more": false}}"#;
        let env_m: BiliEnvelope<InSetData> = serde_json::from_str(json_medias).unwrap();
        assert!(env_m.data.unwrap().medias.is_empty());
    }
}
