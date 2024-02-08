//! Local

use protobuf::MessageFull;

/// Local utils to help read and write
pub trait Local {
    /// The path to the local
    const PATH: &'static str;
    /// Save the resource
    fn save(name: &str, content: impl MessageFull) {
        let path = std::path::PathBuf::from(name).join(name);
        let mut file = std::fs::File::create(path).unwrap();
        content.write_to_writer(&mut file).unwrap();
    }
    /// Remove the resource
    fn remove(name: &str) {
        let path = std::path::PathBuf::from(name).join(name);
        std::fs::remove_file(path).unwrap();
    }
}
