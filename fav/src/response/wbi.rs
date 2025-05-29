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
        fn get_filename(url: &Url) -> &str {
            url.path_segments()
                .and_then(|mut segs| segs.next_back())
                .and_then(|s| s.split('.').next())
                .expect("Bilibili should return a valid wbi")
        }
        format!(
            "{}{}",
            get_filename(&self.img_url),
            get_filename(&self.sub_url)
        )
    }
}
