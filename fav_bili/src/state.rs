use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

macro_rules! impl_state_enum {
    ($enum_name:ident { $($variant:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ::sea_orm::DeriveIden)]
        pub enum $enum_name {
            $($variant),*
        }

        impl $enum_name {
            pub const ALL: &'static [Self] = &[ $(Self::$variant),* ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => stringify!($variant), )*
                }
            }
        }

        impl fmt::Display for $enum_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $enum_name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let normalized: String = s
                    .chars()
                    .filter(|c| !matches!(c, '_' | '-' | ' '))
                    .flat_map(char::to_lowercase)
                    .collect();
                $(
                    if normalized == stringify!($variant).to_ascii_lowercase() {
                        return Ok(Self::$variant);
                    }
                )*
                Err(format!("Invalid {} string: '{s}'", stringify!($enum_name)))
            }
        }

        impl From<$enum_name> for ::sea_orm::Value {
            fn from(value: $enum_name) -> Self {
                ::sea_orm::Value::String(Some(Box::new(value.as_str().to_owned())))
            }
        }
    };
}

impl_state_enum!(AccountState {
    Active,
    Inactive,
    Expired,
});

impl_state_enum!(SetState {
    Active,
    Inactive,
    Unreachable,
});

impl_state_enum!(MediaState {
    Pending,
    Downloading,
    Completed,
    Failed,
    Expired,
    PermissionDenied,
    Skipped,
});

impl_state_enum!(UpState {
    Active,
    Inactive,
    Deactivated,
});

impl_state_enum!(MediaPageState {
    Pending,
    Completed,
    Failed,
});

/// Bilibili media type code stored in `media.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MediaType {
    #[default]
    Video,
    Audio,
    Collection,
    Unknown(u8),
}

impl MediaType {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            2 => Self::Video,
            12 => Self::Audio,
            21 => Self::Collection,
            other => Self::Unknown(other),
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Video => 2,
            Self::Audio => 12,
            Self::Collection => 21,
            Self::Unknown(v) => v,
        }
    }
}

impl From<u8> for MediaType {
    fn from(value: u8) -> Self {
        Self::from_u8(value)
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Video => f.write_str("Video"),
            Self::Audio => f.write_str("Audio"),
            Self::Collection => f.write_str("Collection"),
            Self::Unknown(code) => write!(f, "Unknown({code})"),
        }
    }
}

impl FromStr for MediaType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        match trimmed.to_ascii_lowercase().as_str() {
            "video" | "2" => Ok(Self::Video),
            "audio" | "12" => Ok(Self::Audio),
            "collection" | "21" => Ok(Self::Collection),
            other => {
                if let Some(inner) = other
                    .strip_prefix("unknown(")
                    .and_then(|rest| rest.strip_suffix(')'))
                    && let Ok(code) = inner.parse::<u8>()
                {
                    return Ok(Self::from_u8(code));
                }
                if let Ok(code) = other.parse::<u8>() {
                    return Ok(Self::from_u8(code));
                }
                Err(format!("Invalid MediaType string: '{s}'"))
            }
        }
    }
}

impl From<MediaType> for sea_orm::Value {
    fn from(value: MediaType) -> Self {
        sea_orm::Value::String(Some(Box::new(value.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_display_and_case_insensitive_parse_roundtrip() {
        for &state in MediaState::ALL {
            let s = state.to_string();
            assert_eq!(MediaState::from_str(&s).unwrap(), state);
            assert_eq!(
                MediaState::from_str(&s.to_ascii_lowercase()).unwrap(),
                state
            );
        }
        assert_eq!(
            MediaState::from_str("permission-denied").unwrap(),
            MediaState::PermissionDenied
        );
        assert_eq!(
            MediaState::from_str("permission_denied").unwrap(),
            MediaState::PermissionDenied
        );
        assert_eq!(
            MediaState::from_str("skipped").unwrap(),
            MediaState::Skipped
        );
        assert_eq!(
            MediaPageState::from_str("completed").unwrap(),
            MediaPageState::Completed
        );
        assert!(AccountState::from_str("bogus").is_err());
    }

    #[test]
    fn media_type_roundtrip_and_unknown_preservation() {
        assert_eq!(MediaType::from_u8(2), MediaType::Video);
        assert_eq!(MediaType::from_u8(12), MediaType::Audio);
        assert_eq!(MediaType::from_u8(21), MediaType::Collection);
        assert_eq!(MediaType::from_u8(99), MediaType::Unknown(99));
        assert_eq!(MediaType::Unknown(99).as_u8(), 99);
        assert_eq!(MediaType::Unknown(99).to_string(), "Unknown(99)");
        assert_eq!(
            MediaType::from_str("Unknown(99)").unwrap(),
            MediaType::Unknown(99)
        );
        assert_eq!(MediaType::from_str("video").unwrap(), MediaType::Video);
    }
}
