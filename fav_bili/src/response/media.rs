use core::fmt;
use serde::Deserialize;

use super::Up;

#[derive(Debug, Deserialize)]
pub struct MediaInfoResp {
    pub code: i64,
    pub data: Option<MediaInfoData>,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MediaInfoData {
    pub owner: Up,
    pub pages: Vec<Page>,
    pub staff: Option<Vec<Up>>,
}

#[derive(Debug, Deserialize)]
pub struct Page {
    pub cid: i64,
    pub page: i64,
    pub part: String,
}

#[derive(Debug, Deserialize)]
pub struct Media {
    #[serde(alias = "aid")]
    pub id: i64,
    #[serde(rename = "bvid")]
    pub bv_id: String,
    pub title: String,
    #[serde(default)]
    pub r#type: MediaType,
}

#[derive(Debug, Default, Deserialize)]
#[repr(u8)]
#[serde(from = "u8")]
pub enum MediaType {
    #[default]
    Video = 2,
    Audio = 12,
    Collection = 21,
}

impl From<u8> for MediaType {
    fn from(value: u8) -> Self {
        match value {
            2 => Self::Video,
            12 => Self::Audio,
            21 => Self::Collection,
            _ => Self::Video,
        }
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Video => write!(f, "Video"),
            Self::Audio => write!(f, "Audio"),
            Self::Collection => write!(f, "Collection"),
        }
    }
}
