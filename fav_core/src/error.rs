//! Core error

use std::io;

/// Fav Core's error enum.
#[derive(Debug)]
pub enum FavCoreError {
    /// AuthError: failed to login or logout.
    AuthError,
    /// ParamsError: The params provided to API cannot meet the demand.
    ParamsError(String),
    /// NetworkError: The network error.
    NetworkError(reqwest::Error),
    /// Ctrl-C cancelled
    Cancel,
    /// UtilError: The error from util.
    UtilsError(Box<dyn std::error::Error + Send>),
    /// The error from serde_json
    SerdeError(serde_json::Error),
    /// The error from protobuf_json_mapping
    Json2ProtobufError(protobuf_json_mapping::ParseError),
    /// The error from protobuf
    ProtobufError(protobuf::Error),
    /// IO error
    IoError(std::io::Error),
}

impl std::fmt::Display for FavCoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FavCoreError::AuthError => write!(f, "AuthErr: failed to login or logout."),
            FavCoreError::ParamsError(msg) => write!(f, "{}", msg),
            FavCoreError::NetworkError(source) => write!(f, "NetworkErr:: {}", source),
            FavCoreError::Cancel => write!(f, "Ctrl-C cancelled"),
            FavCoreError::UtilsError(source) => write!(f, "UtilsErr: {}", source),
            FavCoreError::SerdeError(source) => write!(f, "SerdeErr:: {}", source),
            FavCoreError::Json2ProtobufError(source) => write!(f, "ProtobufParseErr: {}", source),
            FavCoreError::ProtobufError(source) => write!(f, "ProtobufError: {}", source),
            FavCoreError::IoError(source) => write!(f, "IOErr: {}", source),
        }
    }
}

impl From<reqwest::Error> for FavCoreError {
    fn from(err: reqwest::Error) -> Self {
        FavCoreError::NetworkError(err)
    }
}

impl From<serde_json::Error> for FavCoreError {
    fn from(err: serde_json::Error) -> Self {
        FavCoreError::SerdeError(err)
    }
}

impl From<protobuf_json_mapping::ParseError> for FavCoreError {
    fn from(err: protobuf_json_mapping::ParseError) -> Self {
        FavCoreError::Json2ProtobufError(err)
    }
}

impl From<protobuf::Error> for FavCoreError {
    fn from(err: protobuf::Error) -> Self {
        FavCoreError::ProtobufError(err)
    }
}

impl From<io::Error> for FavCoreError {
    fn from(err: io::Error) -> Self {
        FavCoreError::IoError(err)
    }
}

impl std::error::Error for FavCoreError {}

/// Fav Core's result type.
pub type FavCoreResult<T> = Result<T, FavCoreError>;
