use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MediaInfoPayload {
    pub aid: i64,
}

impl MediaInfoPayload {
    pub fn to_wbi_params(&self) -> BTreeMap<String, String> {
        let mut params = BTreeMap::new();
        params.insert("aid".to_string(), self.aid.to_string());
        params
    }
}
