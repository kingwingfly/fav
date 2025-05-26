//! Local,
//! helping persisting protobuf

use crate::{ops::Net, res::Res, FavCoreResult};
use core::future::Future;
use protobuf::MessageFull;
use std::path::PathBuf;
use url::Url;

/// Refer to a path on disk;
/// impl `PathInfo` for `T: MessageFull` will auto implement [`ProtoLocal`].
pub trait PathInfo {
    /// The path (the Parent directory will be created if not exists)
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
///     const PATH: &'static str = concat!(env!("OUT_DIR"), "/fav_cache/msg");
/// }
/// // trait `ProtoLocal` will be auto implemented for `T: PathInfo + MessageFull`
/// let msg = Msg::default();
/// msg.clone().write();   // The Msg will be write to `fav_cache/msg`
/// let msg_read: Msg = Msg::read().unwrap();
/// assert_eq!(msg, msg_read);
/// Msg::remove();
/// ```
pub trait ProtoLocal: PathInfo + MessageFull {
    /// Write the protobuf to file, which is at `PathInfo::PATH`
    /// Create the parent directory if not exists
    fn write(&self) -> FavCoreResult<()> {
        let path = PathBuf::from(Self::PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(path)?;
        self.write_to_writer(&mut file)?;
        Ok(())
    }

    /// Read the protobuf from file, which is at `PathInfo::PATH`
    fn read() -> FavCoreResult<Self> {
        let path = PathBuf::from(Self::PATH);
        let mut file = std::fs::File::open(path)?;
        Ok(Self::parse_from_reader(&mut file)?)
    }

    /// Remove the resource, which is at `PathInfo::PATH`
    fn remove() {
        let path = PathBuf::from(Self::PATH);
        std::fs::remove_file(path).ok(); // Just omit the result
    }
}

impl<T> ProtoLocal for T where T: PathInfo + MessageFull {}

/// Make it able to save the resource to local
pub trait SaveLocal: Net {
    /// Save the resource to local.
    /// `cancelled: Future<...>`, if Future is ready, one can cleanup and
    /// shutdown gracefully, then return `FavCoreError::Cancel`.
    fn download<R, Fut, Any>(
        &self,
        res: &mut R,
        urls: Vec<Url>,
        cancelled: Fut,
    ) -> impl Future<Output = FavCoreResult<()>>
    where
        R: Res,
        Fut: Future<Output = Any> + Send,
        Any: Send;
}
