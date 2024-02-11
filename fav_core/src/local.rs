//! Local

use crate::{
    api::ApiProvider,
    attr::{ResAttr, ResSetAttr},
    FavCoreResult,
};
use protobuf::MessageFull;

/// Local auth
pub trait Auth: ApiProvider + ProtoLocal {
    /// Login
    fn login(&self) -> FavCoreResult<()>;
    /// Logout
    fn logout(&self) -> FavCoreResult<()>;
}

/// Local fetch
pub trait Fetch: ApiProvider + ProtoLocal {
    /// Fetch resources
    fn fetch(&self, resources: &mut impl ResSetAttr);
    /// Fetch one resource
    fn fetch_one(&self, resource: &mut impl ResAttr);
}

/// Local pull
pub trait Pull: ApiProvider + ResLocal {
    /// Pull the resources
    fn pull(&self, resource: &impl ResSetAttr) -> FavCoreResult<()>;
    /// Pull one resource
    fn pull_one(&self, resource: &impl ResAttr) -> FavCoreResult<()>;
}

/// Protobuf local/persist utils to read and write
/// # Example
/// ```
/// # use fav_core::local::ProtoLocal;
/// use fav_test_utils::VideoMeta;
/// // The meta manager
/// struct MetaLocal;
///
/// impl ProtoLocal for MetaLocal {
///     const PATH: &'static str = ".fav";
/// }
///
/// # fn main() {
/// let v = VideoMeta::default();
/// MetaLocal::write("video", &v);
/// let read_v: VideoMeta = MetaLocal::read("video");
/// assert_eq!(v, read_v);
/// MetaLocal::remove("video");
/// # }
/// ```
pub trait ProtoLocal {
    /// The path to the local protobuf dir
    const PATH: &'static str;

    /// Write the protobuf
    fn write<T: MessageFull>(name: &str, content: &T) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        let mut file = std::fs::File::create(path).unwrap();
        content.write_to_writer(&mut file).unwrap();
    }

    /// Read the protobuf
    fn read<T: MessageFull>(name: &str) -> T {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        let mut file = std::fs::File::open(path).unwrap();
        T::parse_from_reader(&mut file).unwrap()
    }

    /// Remove the resource
    fn remove(name: &str) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        std::fs::remove_file(path).unwrap();
    }
}

/// Making resources able to be locally managed
pub trait ResLocal {}
