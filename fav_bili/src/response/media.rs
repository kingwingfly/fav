use serde::{Deserialize, Deserializer};

pub use crate::state::MediaType;

use super::{Up, null_as_default};

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
pub struct Media {
    #[serde(alias = "aid")]
    pub id: i64,
    #[serde(rename = "bvid")]
    pub bv_id: String,
    pub title: String,
    #[serde(
        default = "default_media_type",
        deserialize_with = "deserialize_media_type"
    )]
    pub r#type: MediaType,
    /// Favorite-item attribute bitmask (`attr != 0`, e.g. `9`, marks an invalidated/deleted video).
    #[serde(default)]
    pub attr: i64,
    /// Present in `/x/v3/fav/resource/list` responses so UP ownership can be populated without `/view`.
    #[serde(default)]
    pub upper: Option<Up>,
    #[serde(default)]
    pub fav_time: i64,
    #[serde(default)]
    pub created: i64,
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
