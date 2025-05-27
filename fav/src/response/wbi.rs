use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
pub struct WbiResp {
    pub data: WbiData,
}

#[derive(Debug, Deserialize)]
pub struct WbiData {
    pub mid: u64,
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
                .unwrap()
                .next_back()
                .unwrap()
                .split(".")
                .next()
                .unwrap(),
            self.sub_url
                .path_segments()
                .unwrap()
                .next_back()
                .unwrap()
                .split(".")
                .next()
                .unwrap(),
        )
    }
}
