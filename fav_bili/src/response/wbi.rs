use anyhow::Result;
use serde::Deserialize;
use url::Url;

use crate::wbi::{extract_key_from_url, mixin_key};

/// Response payload of `/x/web-interface/nav`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountInfo {
    #[serde(rename = "isLogin", default)]
    pub is_login: bool,
    #[serde(default)]
    pub mid: Option<i64>,
    #[serde(default)]
    pub uname: Option<String>,
    #[serde(default)]
    pub wbi_img: Option<Wbi>,
}

/// Alias for backwards clarity when callers refer to `/x/web-interface/nav` as `WbiData`.
pub type WbiData = AccountInfo;

#[derive(Debug, Clone, Deserialize)]
pub struct Wbi {
    pub img_url: Url,
    pub sub_url: Url,
}

impl Wbi {
    pub fn img_key(&self) -> Result<String> {
        extract_key_from_url(&self.img_url)
    }

    pub fn sub_key(&self) -> Result<String> {
        extract_key_from_url(&self.sub_url)
    }

    pub fn key(&self) -> Result<String> {
        Ok(format!("{}{}", self.img_key()?, self.sub_key()?))
    }

    pub fn mixin_key(&self) -> Result<String> {
        mixin_key(&self.img_key()?, &self.sub_key()?)
    }
}
