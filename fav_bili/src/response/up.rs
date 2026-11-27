use serde::Deserialize;

use super::{null_as_default, pick_optional_str};

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "RawUp")]
pub struct Up {
    pub mid: i64,
    pub name: String,
}

#[derive(Deserialize)]
struct RawUp {
    mid: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    uname: Option<String>,
}

impl TryFrom<RawUp> for Up {
    type Error = &'static str;

    fn try_from(raw: RawUp) -> Result<Self, Self::Error> {
        let name =
            pick_optional_str(raw.name, raw.uname).ok_or("missing `name` / `uname` field in UP")?;
        Ok(Self { mid: raw.mid, name })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FollowingUpData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<Up>,
    #[serde(default)]
    pub total: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FollowingNumData {
    pub following: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishNumData {
    pub video: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_handles_dual_name_and_uname_keys() {
        let both = r#"{"mid": 100, "name": "Alice", "uname": "AliceLegacy"}"#;
        let up: Up = serde_json::from_str(both).unwrap();
        assert_eq!(up.mid, 100);
        assert_eq!(up.name, "Alice");

        let empty_name = r#"{"mid": 101, "name": "", "uname": "Bob"}"#;
        let up: Up = serde_json::from_str(empty_name).unwrap();
        assert_eq!(up.name, "Bob");

        let name_only = r#"{"mid": 102, "name": "Carol"}"#;
        let up: Up = serde_json::from_str(name_only).unwrap();
        assert_eq!(up.name, "Carol");

        let uname_only = r#"{"mid": 103, "uname": "Dave"}"#;
        let up: Up = serde_json::from_str(uname_only).unwrap();
        assert_eq!(up.name, "Dave");

        let missing_both = r#"{"mid": 104}"#;
        assert!(serde_json::from_str::<Up>(missing_both).is_err());
    }
}
