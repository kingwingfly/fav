//! Local

use crate::{
    api::ApiProvider,
    attr::{ResAttr, ResSetAttr},
    FavCoreResult,
};
use protobuf::MessageFull;

/// Local auth
pub trait Auth: ApiProvider + LocalMeta {
    /// Login
    fn login(&self) -> FavCoreResult<()>;
    /// Logout
    fn logout(&self) -> FavCoreResult<()>;
}

/// Local fetch
pub trait Fetch: ApiProvider + LocalMeta {
    /// Fetch resources
    fn fetch(&self, resources: impl ResSetAttr);
    /// Fetch one resource
    fn fetch_one(&self, resource: impl ResAttr);
}

/// Local pull
pub trait Pull: ApiProvider + LocalRes {
    /// Pull the resources
    fn pull(&self, resource: impl ResSetAttr) -> FavCoreResult<()>;
    /// Pull one resource
    fn pull_one(&self, resource: impl ResAttr) -> FavCoreResult<()>;
}

/// This trait makes metadatas able to be locally managed
pub trait LocalMeta: Local {}

/// This trait makes resources able to be locally managed
pub trait LocalRes: Local {}

/// Local utils to help read and write
pub trait Local {
    /// The path to the local
    const PATH: &'static str;

    /// Save the resource
    fn save(name: &str, content: impl MessageFull) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        let mut file = std::fs::File::create(path).unwrap();
        content.write_to_writer(&mut file).unwrap();
    }

    /// Remove the resource
    fn remove(name: &str) {
        let path = std::path::PathBuf::from(Self::PATH).join(name);
        std::fs::remove_file(path).unwrap();
    }
}
