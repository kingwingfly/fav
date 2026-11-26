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
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct BiliEnvelope<T> {
    pub code: i64,
    #[serde(default, alias = "msg")]
    pub message: Option<String>,
    #[serde(default = "option_none")]
    pub data: Option<T>,
}

fn option_none<T>() -> Option<T> {
    None
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
    fn null_as_default_deserializes_null_arrays_as_empty() {
        let json = r#"{"code": 0, "data": {"list": null}}"#;
        let env: BiliEnvelope<ListSetData> = serde_json::from_str(json).unwrap();
        assert!(env.data.unwrap().list.is_empty());

        let json_medias = r#"{"code": 0, "data": {"medias": null, "has_more": false}}"#;
        let env_m: BiliEnvelope<InSetData> = serde_json::from_str(json_medias).unwrap();
        assert!(env_m.data.unwrap().medias.is_empty());
    }
}
