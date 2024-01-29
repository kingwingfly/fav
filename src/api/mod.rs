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
pub(crate) mod pull;
pub(crate) mod status;
pub(crate) mod track;
pub(crate) mod untrack;

#[allow(unused)]
pub(super) const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.3 Safari/605.1.15";
pub(super) const REFERER: &str = "https://www.bilibili.com/";
static PARSE_OPTIONS: ParseOptions = ParseOptions {
    ignore_unknown_fields: true,
    _future_options: (),
};

fn client() -> &'static Client {
    use reqwest::header::{self, HeaderMap};

    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("SESSDATA={}", config().cookie.SESSDATA)
                .parse()
                .unwrap(),
        );
        headers.insert(header::USER_AGENT, USER_AGENT.parse().unwrap());
        headers.insert(header::REFERER, REFERER.parse().unwrap());
        Client::builder().default_headers(headers).build().unwrap()
    })
}

fn parse_message<M: MessageFull>(json: &serde_json::Value) -> M {
    parse_from_str_with_options(&json.to_string(), &PARSE_OPTIONS).unwrap()
}

pub(super) fn timestamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
