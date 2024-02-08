//! helper functions for the CLI

#![deny(missing_docs, rustdoc::broken_intra_doc_links)]
#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

mod api;
mod cli;
mod config;
mod daemon;
mod meta;
mod proto;

pub use cli::Cli;
