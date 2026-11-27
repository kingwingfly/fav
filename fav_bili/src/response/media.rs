use serde::{Deserialize, Deserializer};

pub use crate::state::MediaType;

use super::{Up, null_as_default, pick_optional_str};

#[derive(Debug, Clone, Deserialize)]
pub struct MediaInfoData {
    #[serde(default)]
    pub aid: i64,
    #[serde(default)]
    pub bvid: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub pic: String,
    #[serde(default)]
    pub pubdate: i64,
    #[serde(default)]
    pub duration: i64,
    pub owner: Up,
    #[serde(default, deserialize_with = "null_as_default")]
    pub pages: Vec<Page>,
    #[serde(default)]
    pub staff: Option<Vec<Up>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Page {
    pub cid: i64,
    pub page: i64,
    #[serde(default)]
    pub part: String,
    #[serde(default)]
    pub duration: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "RawMedia")]
pub struct Media {
    pub id: i64,
    pub bv_id: String,
    pub title: String,
    pub r#type: MediaType,
    /// Favorite-item attribute bitmask (`attr != 0`, e.g. `9`, marks an invalidated/deleted video).
    pub attr: i64,
    /// Present in `/x/v3/fav/resource/list` responses so UP ownership can be populated without `/view`.
    pub upper: Option<Up>,
    pub fav_time: i64,
    pub created: i64,
}

#[derive(Deserialize)]
struct RawMedia {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    aid: Option<i64>,
    #[serde(default)]
    bvid: Option<String>,
    #[serde(default)]
    bv_id: Option<String>,
    title: String,
    #[serde(
        default = "default_media_type",
        deserialize_with = "deserialize_media_type"
    )]
    r#type: MediaType,
    #[serde(default)]
    attr: i64,
    #[serde(default)]
    upper: Option<Up>,
    #[serde(default)]
    fav_time: i64,
    #[serde(default)]
    created: i64,
}

impl TryFrom<RawMedia> for Media {
    type Error = &'static str;

    fn try_from(raw: RawMedia) -> Result<Self, Self::Error> {
        let id = raw
            .id
            .or(raw.aid)
            .ok_or("missing `id` / `aid` field in media")?;
        let bv_id = pick_optional_str(raw.bvid, raw.bv_id)
            .ok_or("missing `bvid` / `bv_id` field in media")?;
        Ok(Self {
            id,
            bv_id,
            title: raw.title,
            r#type: raw.r#type,
            attr: raw.attr,
            upper: raw.upper,
            fav_time: raw.fav_time,
            created: raw.created,
        })
    }
}

impl Media {
    pub fn is_invalidated(&self) -> bool {
        self.attr != 0 || self.title == "已失效视频"
    }
}

fn default_media_type() -> MediaType {
    MediaType::Video
}

fn deserialize_media_type<'de, D>(deserializer: D) -> Result<MediaType, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<u8>::deserialize(deserializer)?;
    Ok(raw.map(MediaType::from_u8).unwrap_or(MediaType::Video))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_handles_dual_id_aid_and_bvid_bv_id_keys() {
        let both = r#"{
            "id": 170001,
            "aid": 170001,
            "bvid": "BV17x411w7KC",
            "bv_id": "BV17x411w7KC",
            "title": "Dual Key Video",
            "type": 2,
            "attr": 0,
            "upper": { "mid": 42, "name": "Creator", "uname": "CreatorLegacy" }
        }"#;
        let media: Media = serde_json::from_str(both).unwrap();
        assert_eq!(media.id, 170001);
        assert_eq!(media.bv_id, "BV17x411w7KC");
        assert_eq!(media.title, "Dual Key Video");
        assert_eq!(media.r#type, MediaType::Video);
        assert_eq!(media.upper.as_ref().unwrap().name, "Creator");

        let aid_and_bvid = r#"{"aid": 170002, "bvid": "BV1xx411c7mD", "title": "Archive Item"}"#;
        let media: Media = serde_json::from_str(aid_and_bvid).unwrap();
        assert_eq!(media.id, 170002);
        assert_eq!(media.bv_id, "BV1xx411c7mD");

        let id_and_bv_id = r#"{"id": 170003, "bv_id": "BV1ab411c7mD", "title": "Fav Item"}"#;
        let media: Media = serde_json::from_str(id_and_bv_id).unwrap();
        assert_eq!(media.id, 170003);
        assert_eq!(media.bv_id, "BV1ab411c7mD");

        let empty_bvid_fallback =
            r#"{"id": 170004, "bvid": "", "bv_id": "BV1cd411c7mD", "title": "Fallback BV"}"#;
        let media: Media = serde_json::from_str(empty_bvid_fallback).unwrap();
        assert_eq!(media.bv_id, "BV1cd411c7mD");

        let missing_id = r#"{"bvid": "BV1xx411c7mD", "title": "No ID"}"#;
        assert!(serde_json::from_str::<Media>(missing_id).is_err());

        let missing_bvid = r#"{"id": 170005, "title": "No BV"}"#;
        assert!(serde_json::from_str::<Media>(missing_bvid).is_err());
    }
}
