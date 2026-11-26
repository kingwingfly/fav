use std::collections::BTreeMap;

/// Default DASH `fnval`: DASH (16) | HDR (64) | 4K (128) | Dolby Audio (256) | Dolby Vision (512)
/// | 8K (1024) | AV1 (2048).
pub const DEFAULT_DASH_FNVAL: u32 = 16 | 64 | 128 | 256 | 512 | 1024 | 2048;

#[derive(Debug, Clone)]
pub struct DashPayload {
    pub avid: i64,
    pub cid: i64,
    pub qn: u32,
    pub fnval: u32,
    pub fnver: u8,
    pub fourk: u8,
}

impl DashPayload {
    pub fn new(avid: i64, cid: i64) -> Self {
        Self {
            avid,
            cid,
            qn: 127,
            fnval: DEFAULT_DASH_FNVAL,
            fnver: 0,
            fourk: 1,
        }
    }

    pub fn with_qn(mut self, qn: u32) -> Self {
        self.qn = qn;
        self
    }

    pub fn to_wbi_params(&self) -> BTreeMap<String, String> {
        let mut params = BTreeMap::new();
        params.insert("avid".to_string(), self.avid.to_string());
        params.insert("cid".to_string(), self.cid.to_string());
        params.insert("fnval".to_string(), self.fnval.to_string());
        params.insert("fnver".to_string(), self.fnver.to_string());
        params.insert("fourk".to_string(), self.fourk.to_string());
        params.insert("qn".to_string(), self.qn.to_string());
        params
    }
}
