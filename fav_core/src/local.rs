//! Local

use crate::{
    api::ApiProvider,
    attr::ResAttr,
    relation::{ResRel, ResSetRel},
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
    /// Fetch one resource
    fn fetch<R>(&self, resource: &mut impl ResRel);
    /// Fetch resources
    fn fetch_all<R>(&self, resources: &mut impl ResSetRel<R>)
    where
        R: ResAttr,
    {
        for _r in resources.resources() {}
    }
}

/// Local pull
pub trait Pull: ApiProvider + ResLocal {
    /// Pull one resource
    fn pull<R>(&self, resource: &impl ResRel) -> FavCoreResult<()>;
    /// Pull the resources
    fn pull_all<R>(&self, resource: &impl ResSetRel<R>) -> FavCoreResult<()>
    where
        R: ResAttr;
}

/// Has path on disk
pub trait PathInfo {
    /// The path
    const PATH: &'static str;
}

/// Protobuf local/persist utils to read and write
/// # Example
/// ```
/// # #[path = "test_utils/mod.rs"]
/// # mod test_utils;
/// # use test_utils::data::Msg;
/// use fav_core::local::{ProtoLocal, PathInfo};
///
/// // Require `Msg` to implemente `protobuf::MessageFull`
/// impl PathInfo for Msg {
///     const PATH: &'static str = "temp";
/// }
/// // trait `ProtoLocal` will be auto implemented for `T: PathInfo + MessageFull`
/// # fn main() {
/// let msg = Msg::default();
/// msg.clone().write("msg");   // The Msg will be write to `.fav/msg`
/// let msg_read: Msg = Msg::read("msg");
/// assert_eq!(msg, msg_read);
/// Msg::remove("msg");
/// # }
/// ```
pub trait ProtoLocal: PathInfo + MessageFull {
    /// Write the protobuf to file, which is at `PathInfo::PATH + name`
    fn write(self, name: &str) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        let mut file = std::fs::File::create(path).unwrap();
        self.write_to_writer(&mut file).unwrap();
    }

    /// Read the protobuf from file, which is at `PathInfo::PATH + name`
    fn read(name: &str) -> Self {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        let mut file = std::fs::File::open(path).unwrap();
        Self::parse_from_reader(&mut file).unwrap()
    }

    /// Remove the resource, which is at `PathInfo::PATH + name`
    fn remove(name: &str) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        std::fs::remove_file(path).ok(); // Just omit the result
    }
}

impl<T> ProtoLocal for T where T: PathInfo + MessageFull {}

/// Making resources able to be locally managed
pub trait ResLocal: PathInfo {
    /// Write the protobuf
    fn write(name: &str, contents: impl AsRef<[u8]>) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        std::fs::write(path, contents).unwrap();
    }

    /// Remove the resource
    fn remove(name: &str) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        std::fs::remove_file(path).unwrap();
    }
}
