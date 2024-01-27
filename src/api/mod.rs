//! The api module.

use crate::config::config;
use protobuf::MessageFull;
use protobuf_json_mapping::{parse_from_str_with_options, ParseOptions};
use reqwest::Client;
use std::sync::OnceLock;

pub(crate) mod auth;
mod error;
pub(crate) mod fetch;
pub(crate) mod init;
pub(crate) mod like;
pub(crate) mod status;
pub(crate) mod track;
pub(crate) mod untrack;

static PARSE_OPTIONS: ParseOptions = ParseOptions {
    ignore_unknown_fields: true,
    _future_options: (),
};

fn client() -> &'static Client {
    use reqwest::header::{HeaderMap, COOKIE};

    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            format!("SESSDATA={}", config().cookie.SESSDATA)
                .parse()
                .unwrap(),
        );
        Client::builder().default_headers(headers).build().unwrap()
    })
}

fn parse_message<M: MessageFull>(json: &serde_json::Value) -> M {
    parse_from_str_with_options(&json.to_string(), &PARSE_OPTIONS).unwrap()
}
