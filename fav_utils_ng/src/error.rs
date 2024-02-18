//! Error types for `fav_utils`

use fav_core::FavCoreError;

/// Error type for `fav_utils`
#[derive(Debug)]
pub enum FavUtilsError {
    /// Login error
    LoginError,
    /// Login error
    LogoutError,
    /// No cookie. This error will be returned when the cookie is not found.
    NoCookie,
    /// Qr code expired
    QrExpired,
    /// FFMPEG merge failed
    MergeFailed,
}

impl std::error::Error for FavUtilsError {}

impl std::fmt::Display for FavUtilsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FavUtilsError::LoginError => write!(f, "Login error"),
            FavUtilsError::LogoutError => write!(f, "Logout error"),
            FavUtilsError::NoCookie => write!(f, "No cookie"),
            FavUtilsError::QrExpired => write!(f, "Qr code expired"),
            FavUtilsError::MergeFailed => write!(f, "FFMPEG merge failed"),
        }
    }
}

impl From<FavUtilsError> for FavCoreError {
    fn from(err: FavUtilsError) -> Self {
        FavCoreError::UtilsError(Box::new(err))
    }
}

/// Result type for `fav_utils`
pub type FavUtilsResult<T> = Result<T, FavUtilsError>;
