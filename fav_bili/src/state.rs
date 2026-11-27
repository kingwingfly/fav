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

impl_state_enum!(SetKind {
    Created,
    Collected,
    Season,
});

#[allow(clippy::derivable_impls)]
impl Default for SetKind {
    fn default() -> Self {
        Self::Created
    }
}

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

/// Desired video clarity / quality cap for Bilibili DASH and legacy playurl streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VideoClarity {
    #[default]
    Best,
    EightK,
    Dolby,
    Hdr,
    FourK,
    P1080HighFrame,
    P1080Plus,
    AiEnhanced,
    P1080,
    P720HighFrame,
    P720,
    P480,
    P360,
    P240,
    Worst,
}

impl VideoClarity {
    /// Canonical lowercase names suitable for CLI help text and shell completion hints.
    pub const ALL_NAMES: &'static [&'static str] = &[
        "best", "8k", "dolby", "hdr", "4k", "1080p60", "1080p+", "ai", "1080p", "720p60", "720p",
        "480p", "360p", "240p", "worst",
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::EightK => "8k",
            Self::Dolby => "dolby",
            Self::Hdr => "hdr",
            Self::FourK => "4k",
            Self::P1080HighFrame => "1080p60",
            Self::P1080Plus => "1080p+",
            Self::AiEnhanced => "ai",
            Self::P1080 => "1080p",
            Self::P720HighFrame => "720p60",
            Self::P720 => "720p",
            Self::P480 => "480p",
            Self::P360 => "360p",
            Self::P240 => "240p",
            Self::Worst => "worst",
        }
    }

    /// Upper-bound Bilibili `qn` code for tiered caps (`None` for `Best` and `Worst`).
    pub const fn max_qn(&self) -> Option<u32> {
        match self {
            Self::Best | Self::Worst => None,
            Self::EightK => Some(127),
            Self::Dolby => Some(126),
            Self::Hdr => Some(125),
            Self::FourK => Some(120),
            Self::P1080HighFrame => Some(116),
            Self::P1080Plus => Some(112),
            Self::AiEnhanced => Some(100),
            Self::P1080 => Some(80),
            Self::P720HighFrame => Some(74),
            Self::P720 => Some(64),
            Self::P480 => Some(32),
            Self::P360 => Some(16),
            Self::P240 => Some(6),
        }
    }

    /// Numeric `qn` parameter to send in playurl requests.
    pub const fn request_qn(&self) -> u32 {
        match self {
            Self::Best => 127,
            Self::Worst => 6,
            other => match other.max_qn() {
                Some(qn) => qn,
                None => 127,
            },
        }
    }
}

impl fmt::Display for VideoClarity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for VideoClarity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "best" | "auto" => Ok(Self::Best),
            "8k" | "127" => Ok(Self::EightK),
            "dolby" | "126" => Ok(Self::Dolby),
            "hdr" | "125" => Ok(Self::Hdr),
            "4k" | "2160p" | "120" => Ok(Self::FourK),
            "1080p60" | "116" => Ok(Self::P1080HighFrame),
            "1080p+" | "1080p_plus" | "1080p-plus" | "112" => Ok(Self::P1080Plus),
            "ai" | "100" => Ok(Self::AiEnhanced),
            "1080p" | "80" => Ok(Self::P1080),
            "720p60" | "74" => Ok(Self::P720HighFrame),
            "720p" | "64" => Ok(Self::P720),
            "480p" | "32" => Ok(Self::P480),
            "360p" | "16" => Ok(Self::P360),
            "240p" | "6" => Ok(Self::P240),
            "worst" | "lowest" => Ok(Self::Worst),
            other => Err(format!(
                "Invalid video clarity '{other}' (expected one of: {})",
                Self::ALL_NAMES.join(", ")
            )),
        }
    }
}

impl Serialize for VideoClarity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VideoClarity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// Preferred Bilibili DASH video codec (`codecid`: AVC=7, HEVC=12, AV1=13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    Avc,
    Hevc,
    Av1,
}

impl VideoCodec {
    pub const ALL_NAMES: &'static [&'static str] = &["avc", "hevc", "av1"];

    pub const fn codecid(&self) -> u32 {
        match self {
            Self::Avc => 7,
            Self::Hevc => 12,
            Self::Av1 => 13,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Avc => "avc",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
        }
    }
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for VideoCodec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "avc" | "h264" | "h.264" | "7" => Ok(Self::Avc),
            "hevc" | "h265" | "h.265" | "12" => Ok(Self::Hevc),
            "av1" | "13" => Ok(Self::Av1),
            other => Err(format!(
                "Invalid video codec '{other}' (expected one of: avc/h264, hevc/h265, av1)"
            )),
        }
    }
}

impl Serialize for VideoCodec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VideoCodec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

impl clap::ValueEnum for VideoCodec {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Avc, Self::Hevc, Self::Av1]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        use clap::builder::PossibleValue;
        Some(match self {
            Self::Avc => PossibleValue::new("avc").alias("h264"),
            Self::Hevc => PossibleValue::new("hevc").alias("h265"),
            Self::Av1 => PossibleValue::new("av1"),
        })
    }
}

/// Output container/codec for `--sound-only` audio extraction.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    #[default]
    Copy,
    M4a,
    Mp3,
    Flac,
    Opus,
}

impl AudioFormat {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Opus => "opus",
        }
    }

    /// Output file extension for explicit formats, or `None` for `Copy` (inferred from stream).
    pub const fn extension(&self) -> Option<&'static str> {
        match self {
            Self::Copy => None,
            Self::M4a => Some("m4a"),
            Self::Mp3 => Some("mp3"),
            Self::Flac => Some("flac"),
            Self::Opus => Some("opus"),
        }
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AudioFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "copy" => Ok(Self::Copy),
            "m4a" => Ok(Self::M4a),
            "mp3" => Ok(Self::Mp3),
            "flac" => Ok(Self::Flac),
            "opus" => Ok(Self::Opus),
            other => Err(format!(
                "Invalid audio format '{other}' (expected one of: copy, m4a, mp3, flac, opus)"
            )),
        }
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
        for &kind in SetKind::ALL {
            let s = kind.to_string();
            assert_eq!(SetKind::from_str(&s).unwrap(), kind);
            assert_eq!(SetKind::from_str(&s.to_ascii_lowercase()).unwrap(), kind);
        }
        assert_eq!(SetKind::default(), SetKind::Created);
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
        assert!(SetKind::from_str("bogus").is_err());
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

    #[test]
    fn video_clarity_parsing_qn_and_serde_roundtrip() {
        assert_eq!(VideoClarity::default(), VideoClarity::Best);
        assert_eq!(VideoClarity::ALL_NAMES.len(), 15);
        for &name in VideoClarity::ALL_NAMES {
            let parsed = VideoClarity::from_str(name).unwrap();
            assert_eq!(parsed.to_string(), name);
            let json = serde_json::to_string(&parsed).unwrap();
            let back: VideoClarity = serde_json::from_str(&json).unwrap();
            assert_eq!(back, parsed);
        }

        let cases = [
            ("best", VideoClarity::Best, None, 127),
            ("AUTO", VideoClarity::Best, None, 127),
            ("8k", VideoClarity::EightK, Some(127), 127),
            ("127", VideoClarity::EightK, Some(127), 127),
            ("dolby", VideoClarity::Dolby, Some(126), 126),
            ("126", VideoClarity::Dolby, Some(126), 126),
            ("hdr", VideoClarity::Hdr, Some(125), 125),
            ("125", VideoClarity::Hdr, Some(125), 125),
            ("4K", VideoClarity::FourK, Some(120), 120),
            ("2160p", VideoClarity::FourK, Some(120), 120),
            ("120", VideoClarity::FourK, Some(120), 120),
            ("1080p60", VideoClarity::P1080HighFrame, Some(116), 116),
            ("116", VideoClarity::P1080HighFrame, Some(116), 116),
            ("1080p+", VideoClarity::P1080Plus, Some(112), 112),
            ("1080p_plus", VideoClarity::P1080Plus, Some(112), 112),
            ("112", VideoClarity::P1080Plus, Some(112), 112),
            ("ai", VideoClarity::AiEnhanced, Some(100), 100),
            ("100", VideoClarity::AiEnhanced, Some(100), 100),
            ("1080p", VideoClarity::P1080, Some(80), 80),
            ("80", VideoClarity::P1080, Some(80), 80),
            ("720p60", VideoClarity::P720HighFrame, Some(74), 74),
            ("74", VideoClarity::P720HighFrame, Some(74), 74),
            ("720p", VideoClarity::P720, Some(64), 64),
            ("64", VideoClarity::P720, Some(64), 64),
            ("480p", VideoClarity::P480, Some(32), 32),
            ("32", VideoClarity::P480, Some(32), 32),
            ("360p", VideoClarity::P360, Some(16), 16),
            ("16", VideoClarity::P360, Some(16), 16),
            ("240p", VideoClarity::P240, Some(6), 6),
            ("6", VideoClarity::P240, Some(6), 6),
            ("worst", VideoClarity::Worst, None, 6),
            ("lowest", VideoClarity::Worst, None, 6),
        ];
        for (input, expected, max_qn, req_qn) in cases {
            let parsed = VideoClarity::from_str(input).unwrap();
            assert_eq!(parsed, expected, "input={input}");
            assert_eq!(parsed.max_qn(), max_qn, "max_qn({input})");
            assert_eq!(parsed.request_qn(), req_qn, "request_qn({input})");
        }
        assert!(VideoClarity::from_str("999p").is_err());
        assert!(VideoClarity::from_str("").is_err());
    }

    #[test]
    fn video_codec_and_audio_format_roundtrip() {
        assert_eq!(VideoCodec::from_str("avc").unwrap(), VideoCodec::Avc);
        assert_eq!(VideoCodec::from_str("H264").unwrap(), VideoCodec::Avc);
        assert_eq!(VideoCodec::Avc.codecid(), 7);
        assert_eq!(VideoCodec::from_str("hevc").unwrap(), VideoCodec::Hevc);
        assert_eq!(VideoCodec::from_str("h265").unwrap(), VideoCodec::Hevc);
        assert_eq!(VideoCodec::Hevc.codecid(), 12);
        assert_eq!(VideoCodec::from_str("AV1").unwrap(), VideoCodec::Av1);
        assert_eq!(VideoCodec::Av1.codecid(), 13);
        assert!(VideoCodec::from_str("vp9").is_err());

        for codec in [VideoCodec::Avc, VideoCodec::Hevc, VideoCodec::Av1] {
            let json = serde_json::to_string(&codec).unwrap();
            let back: VideoCodec = serde_json::from_str(&json).unwrap();
            assert_eq!(back, codec);
        }

        assert_eq!(AudioFormat::default(), AudioFormat::Copy);
        assert_eq!(AudioFormat::Copy.extension(), None);
        assert_eq!(AudioFormat::M4a.extension(), Some("m4a"));
        assert_eq!(AudioFormat::Mp3.extension(), Some("mp3"));
        assert_eq!(AudioFormat::Flac.extension(), Some("flac"));
        assert_eq!(AudioFormat::Opus.extension(), Some("opus"));

        for fmt in [
            AudioFormat::Copy,
            AudioFormat::M4a,
            AudioFormat::Mp3,
            AudioFormat::Flac,
            AudioFormat::Opus,
        ] {
            let s = fmt.to_string();
            assert_eq!(AudioFormat::from_str(&s).unwrap(), fmt);
            assert_eq!(AudioFormat::from_str(&s.to_ascii_uppercase()).unwrap(), fmt);
            let json = serde_json::to_string(&fmt).unwrap();
            let back: AudioFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(back, fmt);
        }
        assert!(AudioFormat::from_str("wav").is_err());
    }
}
