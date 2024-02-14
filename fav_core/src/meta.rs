//! Meta,
//! helping reading and writing protobuf metadatas

use crate::local::PathInfo;
use protobuf::MessageFull;
use protobuf_json_mapping::{parse_from_str_with_options, ParseOptions};

static PARSE_OPTIONS: ParseOptions = ParseOptions {
    ignore_unknown_fields: true,
    _future_options: (),
};

/// Meta
pub trait Meta: MessageFull + PathInfo {
    /// Read the meta
    fn read() -> Self {
        let mut file = std::fs::File::open(Self::PATH).unwrap();
        Self::parse_from_reader(&mut file).unwrap()
    }

    /// Write the meta
    fn write(self) {
        let mut file = std::fs::File::create(Self::PATH).unwrap();
        self.write_to_writer(&mut file).unwrap();
    }

    /// Parse from json
    fn parse_msg(json: &impl serde::Serialize) -> Self {
        parse_from_str_with_options(&serde_json::to_string(json).unwrap(), &PARSE_OPTIONS).unwrap()
    }
}
