use serde::Serialize;

use crate::response::Wbi;

const MIXIN_KEY_ENC_TAB: [u8; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

pub struct WbiEncoder;

#[derive(Debug, Serialize)]
pub struct WbiEncoded {
    pub w_rid: String,
}

impl WbiEncoder {
    /// The fields' names in `s` should be in alphabetical order.
    pub fn encode(wbi: Wbi, s: &impl Serialize) -> WbiEncoded {
        let query = serde_urlencoded::to_string(s).unwrap();
        let key = wbi.key();
        let key = MIXIN_KEY_ENC_TAB
            .iter()
            .take(32)
            .map(|&i| key.as_bytes()[i as usize] as char)
            .collect::<String>();
        let w_rid = format!("{:x}", md5::compute(query + &key));
        WbiEncoded { w_rid }
    }
}
