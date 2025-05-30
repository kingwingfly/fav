use core::fmt;
use serde::Deserialize;

use super::Up;

#[derive(Debug, Deserialize)]
pub struct MediaInfoResp {
    pub data: MediaInfoData,
}

#[derive(Debug, Deserialize)]
pub struct MediaInfoData {
    pub owner: Up,
    pub staff: Option<Vec<Up>>,
    pub state: i8,
}

#[derive(Debug, Deserialize)]
pub struct Media {
    pub id: i64,
    pub bv_id: String,
    pub title: String,
    pub r#type: MediaType,
}

#[derive(Debug, Deserialize)]
#[repr(u8)]
#[serde(from = "u8")]
pub enum MediaType {
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
