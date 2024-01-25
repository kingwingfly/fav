//! The api module.

use crate::config::config;
use reqwest::Client;
use std::sync::OnceLock;

mod error;
pub(crate) mod fetch;
pub(crate) mod init;
pub(crate) mod login;
pub(crate) mod status;

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
