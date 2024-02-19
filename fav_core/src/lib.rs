//! Fav Core is a crate for [Fav](https://github.com/kingwingfly/fav),
//! a CLI tool to sync one's favorite remote resources.

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

pub mod api;
pub mod attr;
pub mod config;
pub mod error;
pub mod local;
pub mod meta;
pub mod ops;
pub mod remote;
pub mod res;
pub mod status;

/// Proto data structures used for `fav_core`'s test
#[cfg(test)]
mod test_utils;

pub use error::*;

/// Re-export the most important traits and types
pub mod prelude {
    pub use crate::api::{Api, ApiProvider};
    pub use crate::attr::{Attr, Id};
    pub use crate::config::{Config, HttpConfig};
    pub use crate::error::*;
    pub use crate::local::{PathInfo, ProtoLocal, SaveLocal};
    pub use crate::meta::Meta;
    pub use crate::ops::{AuthOps, Net, Ops, ResOps, SetOps, SetOpsExt, SetsOps};
    pub use crate::res::{Res, Set, Sets};
    pub use crate::status::{Status, StatusFlags};
}
