//! Local,
//! helping persisting protobuf

use protobuf::MessageFull;

use crate::FavCoreResult;

/// Refer to a path on disk;
/// impl `PathInfo` for `T: MessageFull` will auto implement [`ProtoLocal`].
pub trait PathInfo {
    /// The path (it must exists, or it will panic at runtime)
    const PATH: &'static str;
}

/// Protobuf local/persist utils for reading and writing
/// # Example
/// ```
/// # #[path = "test_utils/mod.rs"]
/// # mod test_utils;
/// # use test_utils::data::Msg;
/// use fav_core::local::{ProtoLocal, PathInfo};
///
/// // Require `Msg` to implemente `protobuf::MessageFull`
/// impl PathInfo for Msg {
///     const PATH: &'static str = "temp/msg";
/// }
/// // trait `ProtoLocal` will be auto implemented for `T: PathInfo + MessageFull`
/// let msg = Msg::default();
/// msg.clone().write();   // The Msg will be write to `.fav/msg`
/// let msg_read: Msg = Msg::read().unwrap();
/// assert_eq!(msg, msg_read);
/// Msg::remove();
/// ```
pub trait ProtoLocal: PathInfo + MessageFull {
    /// Write the protobuf to file, which is at `PathInfo::PATH`
    fn write(self) -> FavCoreResult<()> {
        let path = std::path::PathBuf::from(Self::PATH);
        let mut file = std::fs::File::create(path)?;
        self.write_to_writer(&mut file)?;
        Ok(())
    }

    /// Read the protobuf from file, which is at `PathInfo::PATH`
    fn read() -> FavCoreResult<Self> {
        let path = std::path::PathBuf::from(Self::PATH);
        let mut file = std::fs::File::open(path)?;
        Ok(Self::parse_from_reader(&mut file)?)
    }

    /// Remove the resource, which is at `PathInfo::PATH`
    fn remove() {
        let path = std::path::PathBuf::from(Self::PATH);
        std::fs::remove_file(path).ok(); // Just omit the result
    }
}

impl<T> ProtoLocal for T where T: PathInfo + MessageFull {}
