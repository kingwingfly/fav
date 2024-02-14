//! `fav_utils` is a utility library for [Fav](https://github.com/kingwingfly/fav).

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

#[cfg(feature = "bili")]
pub mod bili;
pub mod error;
mod proto;
mod utils;
pub use error::*;
