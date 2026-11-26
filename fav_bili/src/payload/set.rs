use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ListSetPayload {
    pub up_mid: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InSetPayload {
    pub media_id: i64,
    pub pn: i64,
    pub ps: u8,
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
