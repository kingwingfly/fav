use crate::{cli::Kind, proto::data::Cookie};
use protobuf::Message;
use std::sync::OnceLock;
use tracing::{info, warn};

const ERR_HINT: &str = "run `fav init` and `fav auth login` first";
const KIND_PATH: &str = ".fav/kind";
const COOKIE_PATH: &str = ".fav/cookie";
const FFMPEG_PATH: &str = ".fav/ffmpeg";

impl Cookie {
    pub(crate) fn persist(&self) {
        let mut file = std::fs::File::create(COOKIE_PATH).expect(ERR_HINT);
        self.write_to_writer(&mut file).expect(ERR_HINT);
        info!("Cookie persisted");
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct Config {
    pub kind: Kind,
    pub cookie: Cookie,
    pub ffmpeg_path: String,
}

impl Config {
    pub fn new() -> Self {
        let kind = match std::fs::read_to_string(KIND_PATH).expect(ERR_HINT).as_str() {
            #[cfg(feature = "bili")]
            "bili" => Kind::Bili,
            _ => panic!("Unknown kind"),
        };
        let ffmpeg_path = match std::fs::read_to_string(FFMPEG_PATH) {
            Ok(path) => path,
            Err(_) => String::from("ffmpeg"),
        };
        match std::fs::File::open(COOKIE_PATH) {
            Ok(mut file) => {
                let cookie = Cookie::parse_from_reader(&mut file).unwrap();
                Self {
                    kind,
                    cookie,
                    ffmpeg_path,
                }
            }
            Err(_) => Self {
                kind,
                ffmpeg_path,
                cookie: Cookie::default(),
            },
        }
    }
}

pub(crate) fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(Config::new)
}

pub(crate) async fn set_ffmpeg_path(path: String) {
    let status = tokio::process::Command::new(&path)
        .arg("-h")
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    match status.success() {
        true => std::fs::write(FFMPEG_PATH, path).expect(ERR_HINT),
        false => warn!("Invalid ffmpeg path"),
    }
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
