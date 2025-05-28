use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
pub struct WbiResp {
    pub data: WbiData,
}

#[derive(Debug, Deserialize)]
pub struct WbiData {
    pub mid: i32,
    pub uname: String,
    pub wbi_img: Wbi,
}

#[derive(Debug, Deserialize)]
pub struct Wbi {
    pub img_url: Url,
    pub sub_url: Url,
}

impl Wbi {
    pub fn key(self) -> String {
        format!(
            "{}{}",
            self.img_url
                .path_segments()
                .and_then(|mut segs| segs.next_back())
                .and_then(|s| s.split('.').next())
                .unwrap_or(""),
            self.sub_url
                .path_segments()
                .and_then(|mut segs| segs.next_back())
                .and_then(|s| s.split('.').next())
                .unwrap_or("")
        )
    }
}
