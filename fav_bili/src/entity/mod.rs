#[allow(unused)]
mod entity_inner;

pub use entity_inner::*;

use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

pub trait ToTableRecord<const N: usize> {
    fn to_record(self) -> [String; N];
}

fn truncate_cell(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    if s.width_cjk() <= max_cols {
        return s.to_owned();
    }
    if max_cols == 1 {
        return "…".to_owned();
    }
    let budget = max_cols - 1;
    let mut width = 0;
    let mut out = String::new();
    for ch in s.chars() {
        let w = ch.width_cjk().unwrap_or(0);
        if width + w > budget {
            break;
        }
        width += w;
        out.push(ch);
    }
    out.push('…');
    out
}

impl ToTableRecord<3> for account::Model {
    fn to_record(self) -> [String; 3] {
        [self.account_id.to_string(), self.name, self.state]
    }
}

impl ToTableRecord<4> for set::Model {
    fn to_record(self) -> [String; 4] {
        [
            self.set_id.to_string(),
            truncate_cell(&self.name, 20),
            self.count.to_string(),
            self.state,
        ]
    }
}

impl ToTableRecord<5> for media::Model {
    fn to_record(self) -> [String; 5] {
        [
            self.id.to_string(),
            self.bv_id,
            truncate_cell(&self.title, 20),
            self.r#type,
            self.state,
        ]
    }
}

impl ToTableRecord<3> for up::Model {
    fn to_record(self) -> [String; 3] {
        [
            self.up_id.to_string(),
            truncate_cell(&self.name, 20),
            self.state,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_debug_and_serialize_redact_secrets() {
        let model = account::Model {
            account_id: 42,
            name: "alice".to_owned(),
            cookies: "SESSDATA=secret123; bili_jct=csrf456".to_owned(),
            state: "Active".to_owned(),
            refresh_token: Some("refresh_secret_789".to_owned()),
        };

        let dbg = format!("{model:?}");
        assert!(!dbg.contains("secret123"));
        assert!(!dbg.contains("csrf456"));
        assert!(!dbg.contains("refresh_secret_789"));
        assert!(dbg.contains("[REDACTED]"));

        let json = serde_json::to_string(&model).unwrap();
        assert!(!json.contains("secret123"));
        assert!(!json.contains("csrf456"));
        assert!(!json.contains("refresh_secret_789"));
        assert!(!json.contains("cookies"));
        assert!(!json.contains("refresh_token"));
        assert!(json.contains("\"account_id\":42"));
    }
}
