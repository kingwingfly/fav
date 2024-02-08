//! Fav Core is a crate for `Fav`, a CLI tool to sync one's favorite remote resources.

#![deny(missing_docs)]

pub mod api;
pub mod attr;
pub mod config;
pub mod error;
pub mod local;
pub mod meta;
pub mod relation;
pub mod remote;
pub mod status;

pub use error::*;
