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
    pub try_look: bool,
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
            try_look: false,
        }
    }

    pub fn with_qn(mut self, qn: u32) -> Self {
        self.qn = qn;
        self
    }

    pub fn with_try_look(mut self, try_look: bool) -> Self {
        self.try_look = try_look;
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
        if self.try_look {
            params.insert("try_look".to_string(), "1".to_string());
        }
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_payload_emits_try_look_only_when_enabled() {
        let default_payload = DashPayload::new(170_001, 279_786);
        assert!(!default_payload.try_look);
        let default_params = default_payload.to_wbi_params();
        assert_eq!(default_params.get("qn").map(String::as_str), Some("127"));
        assert!(!default_params.contains_key("try_look"));

        let anon_payload = DashPayload::new(170_001, 279_786)
            .with_qn(80)
            .with_try_look(true);
        assert!(anon_payload.try_look);
        let anon_params = anon_payload.to_wbi_params();
        assert_eq!(anon_params.get("qn").map(String::as_str), Some("80"));
        assert_eq!(anon_params.get("try_look").map(String::as_str), Some("1"));
    }
}
