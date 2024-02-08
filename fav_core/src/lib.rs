//! Fav Core is a crate for `Fav`, a CLI tool to sync one's favorite remote resources.

#![deny(missing_docs)]

mod api;
mod attr;
mod config;
mod error;
mod local;
mod meta;
mod remote;
mod status;

pub use error::*;
