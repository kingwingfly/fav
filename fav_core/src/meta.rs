//! Meta

use protobuf::Message;

/// Meta
pub trait Meta: Message {
    /// The path to the meta
    const PATH: &'static str;

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
}
