use crate::{cli::Kind, proto::data::Cookie};
use protobuf::Message;
use std::sync::OnceLock;

const ERR_HINT: &str = "run `backup init` first";
const KIND_PATH: &str = ".backup/kind";
const COOKIE_PATH: &str = ".backup/cookie";

impl Cookie {
    pub(crate) fn persist(&self) {
        let mut file = std::fs::File::create(COOKIE_PATH).expect(ERR_HINT);
        self.write_to_writer(&mut file).expect(ERR_HINT);
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Config {
    pub kind: Kind,
    pub cookie: Cookie,
}

impl Config {
    pub fn new() -> Self {
        let kind = match std::fs::read_to_string(KIND_PATH).expect(ERR_HINT).as_str() {
            #[cfg(feature = "bili")]
            "bili" => Kind::Bili,
            _ => panic!("unknown kind"),
        };
        let mut file = std::fs::File::open(COOKIE_PATH).expect(ERR_HINT);
        let cookie = Cookie::parse_from_reader(&mut file).unwrap();
        Self { kind, cookie }
    }
}

pub(crate) fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(Config::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_test() {
        assert_eq!(config(), config());
    }

    #[test]
    fn show_config() {
        dbg!(config());
    }
}