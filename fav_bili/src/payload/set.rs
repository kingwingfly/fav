use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ListSetPayload {
    pub up_mid: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectedSetsPayload {
    pub up_mid: i64,
    pub pn: i64,
    pub ps: u8,
    pub platform: &'static str,
}

impl CollectedSetsPayload {
    pub fn new(up_mid: i64, pn: i64, ps: u8) -> Self {
        Self {
            up_mid,
            pn,
            ps,
            platform: "web",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InSetPayload {
    pub media_id: i64,
    pub pn: i64,
    pub ps: u8,
    pub order: &'static str,
    pub platform: &'static str,
}

impl InSetPayload {
    pub fn new(media_id: i64, pn: i64, ps: u8) -> Self {
        Self {
            media_id,
            pn,
            ps,
            order: "mtime",
            platform: "web",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SeasonArchivesPayload {
    pub mid: i64,
    pub season_id: i64,
    pub page_num: i64,
    pub page_size: u8,
    pub sort_reverse: bool,
}

impl SeasonArchivesPayload {
    pub fn new(mid: i64, season_id: i64, page_num: i64, page_size: u8, sort_reverse: bool) -> Self {
        Self {
            mid,
            season_id,
            page_num,
            page_size,
            sort_reverse,
        }
    }

    pub fn to_wbi_params(&self) -> BTreeMap<String, String> {
        let mut params = BTreeMap::new();
        params.insert("mid".to_string(), self.mid.to_string());
        params.insert("season_id".to_string(), self.season_id.to_string());
        params.insert("sort_reverse".to_string(), self.sort_reverse.to_string());
        params.insert("page_num".to_string(), self.page_num.to_string());
        params.insert("page_size".to_string(), self.page_size.to_string());
        params.insert("web_location".to_string(), "333.999".to_string());
        params
    }
}

#[derive(Debug, Clone)]
pub struct InUpPayload {
    pub mid: i64,
    pub pn: i64,
    pub ps: u8,
}

impl InUpPayload {
    pub fn new(mid: i64, pn: i64, ps: u8) -> Self {
        Self { mid, pn, ps }
    }

    pub fn to_wbi_params(&self) -> BTreeMap<String, String> {
        let mut params = BTreeMap::new();
        params.insert("mid".to_string(), self.mid.to_string());
        params.insert("pn".to_string(), self.pn.to_string());
        params.insert("ps".to_string(), self.ps.to_string());
        params
    }
}
