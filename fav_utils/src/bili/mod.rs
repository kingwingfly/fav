//! `fav_utils` for [bilibili](https://www.bilibili.com)

mod api;
mod config;
mod local;
mod ops;
mod res;

pub mod data;

pub use data::*;

impl From<Qn> for String {
    fn from(qn: Qn) -> Self {
        match qn {
            Qn::Default => Qn::EightK.into(),
            _ => (qn as i32).to_string(),
        }
    }
}
