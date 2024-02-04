use crate::proto::data::Meta;
use protobuf::Message;

const META_PATH: &str = ".fav/meta";
const ERR_HINT: &str = "run `fav init` first";

impl Meta {
    pub fn read() -> Self {
        match std::fs::File::open(META_PATH) {
            Ok(mut file) => Meta::parse_from_reader(&mut file).unwrap(),
            Err(_) => Meta::default(),
        }
    }

    pub(crate) fn persist(&self) {
        let mut file = std::fs::File::create(META_PATH).expect(ERR_HINT);
        self.write_to_writer(&mut file).expect(ERR_HINT);
    }
}
