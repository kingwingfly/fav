//! Core error

/// Fav Core's error enum.
#[derive(Debug)]
pub enum FavCoreError {
    /// AuthError: failed to login or logout.
    AuthError,
    /// ParamsError: The params provided to API cannot meet the demand.
    ParamsError(String),
}

impl std::fmt::Display for FavCoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FavCoreError::AuthError => write!(f, "AuthError: failed to login or logout."),
            FavCoreError::ParamsError(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for FavCoreError {}

/// Fav Core's result type.
pub type FavCoreResult<T> = Result<T, FavCoreError>;
