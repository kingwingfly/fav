//! Remote trait for remote operations.

use crate::FavCoreResult;

/// Remote Auth
pub trait Auth {
    /// Login
    fn login(&self) -> FavCoreResult<()>;
    /// Logout
    fn logout(&self) -> FavCoreResult<()>;
}
