use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Buvid3Data {
    pub buvid: String,
}
