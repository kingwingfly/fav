use serde::Deserialize;

use crate::state::SetKind;

use super::{Media, Up, null_as_default};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListSetData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<Set>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Set {
    pub id: i64,
    pub media_count: i64,
    pub title: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CollectedSetsData {
    #[serde(default)]
    pub count: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub list: Vec<CollectedSet>,
    #[serde(default)]
    pub has_more: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectedSet {
    pub id: i64,
    #[serde(default)]
    pub mid: i64,
    pub title: String,
    #[serde(default)]
    pub media_count: i64,
    #[serde(default, rename = "type")]
    pub r#type: u8,
    #[serde(default)]
    pub attr: i64,
    #[serde(default)]
    pub state: i64,
    #[serde(default)]
    pub upper: Option<Up>,
}

impl CollectedSet {
    pub fn owner_mid(&self) -> Option<i64> {
        if let Some(ref upper) = self.upper
            && upper.mid > 0
        {
            return Some(upper.mid);
        }
        if self.mid > 0 {
            return Some(self.mid);
        }
        None
    }

    pub fn set_kind(&self) -> SetKind {
        match self.r#type {
            21 => SetKind::Season,
            _ => SetKind::Collected,
        }
    }

    /// Detect invalidated/deleted collected folders or seasons.
    /// Never treat `attr != 0` as invalidated because valid Bilibili folders use bitmasks like `attr = 22`.
    pub fn is_invalidated(&self) -> bool {
        self.state != 0 || self.title == "该合集已失效" || self.title == "已失效合集"
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeasonArchivesData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub archives: Vec<Media>,
    #[serde(default)]
    pub page: Option<SeasonArchivesPage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeasonArchivesPage {
    #[serde(default)]
    pub page_num: i64,
    #[serde(default)]
    pub page_size: i64,
    #[serde(default)]
    pub total: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InSetData {
    #[serde(default, deserialize_with = "null_as_default")]
    pub medias: Vec<Media>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub info: Option<Set>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InUpData {
    #[serde(default)]
    pub list: InUpList,
    #[serde(default)]
    pub page: Option<InUpPage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InUpPage {
    #[serde(default)]
    pub pn: i64,
    #[serde(default)]
    pub ps: i64,
    #[serde(default)]
    pub count: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InUpList {
    #[serde(default, deserialize_with = "null_as_default")]
    pub vlist: Vec<Media>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::BiliEnvelope;
    use crate::state::MediaType;

    #[test]
    fn collected_sets_data_parses_folders_seasons_bitmasks_invalidations_and_null_list() {
        let json = r#"{
            "code": 0,
            "data": {
                "count": 4,
                "has_more": true,
                "list": [
                    {
                        "id": 1001,
                        "mid": 888,
                        "title": "Public Fav Folder",
                        "media_count": 12,
                        "type": 11,
                        "attr": 22,
                        "state": 0,
                        "upper": { "mid": 999, "name": "OwnerA" }
                    },
                    {
                        "id": 2002,
                        "mid": 777,
                        "title": "Rust Series Season",
                        "media_count": 8,
                        "type": 21,
                        "attr": 0,
                        "state": 0,
                        "upper": null
                    },
                    {
                        "id": 3003,
                        "mid": 0,
                        "title": "Removed Season",
                        "media_count": 0,
                        "type": 21,
                        "attr": 0,
                        "state": 1
                    },
                    {
                        "id": 4004,
                        "mid": 555,
                        "title": "该合集已失效",
                        "media_count": 0,
                        "type": 21,
                        "attr": 0,
                        "state": 0
                    }
                ]
            }
        }"#;

        let env: BiliEnvelope<CollectedSetsData> = serde_json::from_str(json).unwrap();
        let data = env.data.unwrap();
        assert_eq!(data.count, 4);
        assert_eq!(data.has_more, Some(true));
        assert_eq!(data.list.len(), 4);

        let folder = &data.list[0];
        assert_eq!(folder.set_kind(), SetKind::Collected);
        assert_eq!(folder.owner_mid(), Some(999));
        assert_eq!(folder.attr, 22);
        assert!(
            !folder.is_invalidated(),
            "valid folder with attr=22 must NOT be treated as invalidated"
        );

        let season = &data.list[1];
        assert_eq!(season.set_kind(), SetKind::Season);
        assert_eq!(season.owner_mid(), Some(777));
        assert!(!season.is_invalidated());

        let invalid_state = &data.list[2];
        assert_eq!(invalid_state.set_kind(), SetKind::Season);
        assert_eq!(invalid_state.owner_mid(), None);
        assert!(invalid_state.is_invalidated());

        let invalid_title = &data.list[3];
        assert!(invalid_title.is_invalidated());

        let invalid_alt_title: CollectedSet =
            serde_json::from_str(r#"{"id": 5005, "title": "已失效合集", "type": 21, "state": 0}"#)
                .unwrap();
        assert!(invalid_alt_title.is_invalidated());

        let null_list = r#"{"code": 0, "data": {"count": 0, "list": null, "has_more": false}}"#;
        let env_null: BiliEnvelope<CollectedSetsData> = serde_json::from_str(null_list).unwrap();
        let data_null = env_null.data.unwrap();
        assert_eq!(data_null.count, 0);
        assert!(data_null.list.is_empty());
        assert_eq!(data_null.has_more, Some(false));
    }

    #[test]
    fn season_archives_data_parses_archives_page_and_null_archives() {
        let json = r#"{
            "code": 0,
            "data": {
                "archives": [
                    {
                        "aid": 170001,
                        "bvid": "BV17x411w7KC",
                        "title": "Episode 2",
                        "pubdate": 1700000100,
                        "duration": 120
                    },
                    {
                        "aid": 170002,
                        "bvid": "BV1xx411c7mD",
                        "title": "Episode 1",
                        "pubdate": 1700000000,
                        "duration": 100
                    }
                ],
                "page": {
                    "page_num": 1,
                    "page_size": 30,
                    "total": 2
                }
            }
        }"#;

        let env: BiliEnvelope<SeasonArchivesData> = serde_json::from_str(json).unwrap();
        let data = env.data.unwrap();
        assert_eq!(data.archives.len(), 2);
        assert_eq!(data.archives[0].id, 170001);
        assert_eq!(data.archives[0].bv_id, "BV17x411w7KC");
        assert_eq!(data.archives[0].title, "Episode 2");
        assert_eq!(data.archives[0].r#type, MediaType::Video);
        let page = data.page.unwrap();
        assert_eq!(page.page_num, 1);
        assert_eq!(page.page_size, 30);
        assert_eq!(page.total, 2);

        let empty_json = r#"{"code": 0, "data": {"archives": null}}"#;
        let empty_env: BiliEnvelope<SeasonArchivesData> = serde_json::from_str(empty_json).unwrap();
        let empty = empty_env.data.unwrap();
        assert!(empty.archives.is_empty());
        assert!(empty.page.is_none());
    }
}
