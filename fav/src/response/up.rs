use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Up {
    pub mid: i64,
    pub name: String,
}
