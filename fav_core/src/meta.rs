use crate::{attr::Resource, status::Status};
use protobuf::Message;

pub trait Meta: Resource + Status + Message {
    const PATH: &'static str;
    fn read() -> Self {
        let mut file = std::fs::File::open(Self::PATH).unwrap();
        Self::parse_from_reader(&mut file).unwrap()
    }
    fn write(self) {
        let mut file = std::fs::File::create(Self::PATH).unwrap();
        self.write_to_writer(&mut file).unwrap();
    }
}
