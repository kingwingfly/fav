//! Remote trait for remote operations.

use crate::{attr::ResSetAttr, FavCoreResult};

/// Remote auth
pub trait Auth {
    /// Login
    fn login(&self) -> FavCoreResult<()>;
    /// Logout
    fn logout(&self) -> FavCoreResult<()>;
}

/// Remote pull
pub trait Pull {
    /// Pull the resource
    fn pull(&self, resource: impl ResSetAttr) -> FavCoreResult<()>;
}
