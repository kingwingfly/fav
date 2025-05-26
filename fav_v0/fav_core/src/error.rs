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
    /// PointerNotFound
    SerdePointerNotFound,
    /// The error from protobuf_json_mapping
    Json2ProtobufError(protobuf_json_mapping::ParseError),
    /// The error from protobuf
    ProtobufError(protobuf::Error),
    /// IO error
    IoError(std::io::Error),
    /// Id not usable with a message to user
    IdNotUsable {
        /// The id of resource unuseable
        id: String,
        /// The message of the error
        msg: String,
    },
}

impl std::fmt::Display for FavCoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hint = match self {
            FavCoreError::AuthError => "AuthErr: failed to login or logout".to_string(),
            FavCoreError::ParamsError(msg) => msg.to_string(),
            FavCoreError::NetworkError(source) => format!("NetworkErr: {}", source),
            FavCoreError::Cancel => "Ctrl-C cancelled".to_string(),
            FavCoreError::UtilsError(source) => format!("UtilsErr: {}", source),
            FavCoreError::SerdeError(source) => format!("SerdeErr: {}", source),
            FavCoreError::SerdePointerNotFound => "SerdeErr: pointer not found".to_string(),
            FavCoreError::Json2ProtobufError(source) => format!("ProtobufParseErr: {}", source),
            FavCoreError::ProtobufError(source) => format!("ProtobufError: {}", source),
            FavCoreError::IoError(source) => {
                format!("IOErr: {}", source)
            }
            FavCoreError::IdNotUsable { id, msg } => {
                format!("ID<{}> not usable;  {}", id, msg)
            }
        };
        write!(f, "{}", hint)
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
