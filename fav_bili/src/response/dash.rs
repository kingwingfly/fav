use serde::Deserialize;
use url::Url;

use crate::state::VideoClarity;

use super::{null_as_default, pick_optional_str};

#[derive(Debug, Clone, Deserialize)]
pub struct DashData {
    #[serde(default)]
    pub quality: u32,
    #[serde(default)]
    pub dash: Option<Dash>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub durl: Vec<Durl>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub accept_quality: Vec<u32>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub accept_description: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Dash {
    #[serde(default, deserialize_with = "null_as_default")]
    pub video: Vec<DashStream>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub audio: Vec<DashStream>,
    #[serde(default)]
    pub dolby: Option<DolbyAudio>,
    #[serde(default)]
    pub flac: Option<FlacAudio>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "RawDashStream")]
pub struct DashStream {
    pub id: u32,
    pub base_url: Url,
    pub backup_url: Vec<Url>,
    pub mime_type: String,
    pub codecs: String,
    pub bandwidth: u64,
    pub width: u32,
    pub height: u32,
    pub codecid: u32,
}

#[derive(Deserialize)]
struct RawDashStream {
    #[serde(default, deserialize_with = "null_as_default")]
    id: u32,
    #[serde(default)]
    base_url: Option<Url>,
    #[serde(default, rename = "baseUrl")]
    base_url_camel: Option<Url>,
    #[serde(default, deserialize_with = "null_as_default")]
    backup_url: Vec<Url>,
    #[serde(default, rename = "backupUrl", deserialize_with = "null_as_default")]
    backup_url_camel: Vec<Url>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default, rename = "mimeType")]
    mime_type_camel: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    codecs: String,
    #[serde(default, deserialize_with = "null_as_default")]
    bandwidth: u64,
    #[serde(default, deserialize_with = "null_as_default")]
    width: u32,
    #[serde(default, deserialize_with = "null_as_default")]
    height: u32,
    #[serde(default, deserialize_with = "null_as_default")]
    codecid: u32,
}

impl TryFrom<RawDashStream> for DashStream {
    type Error = &'static str;

    fn try_from(raw: RawDashStream) -> Result<Self, Self::Error> {
        let base_url = raw
            .base_url
            .or(raw.base_url_camel)
            .ok_or("missing `base_url` / `baseUrl` field in DASH stream")?;
        let backup_url = if !raw.backup_url.is_empty() {
            raw.backup_url
        } else {
            raw.backup_url_camel
        };
        let mime_type = pick_optional_str(raw.mime_type, raw.mime_type_camel).unwrap_or_default();
        Ok(Self {
            id: raw.id,
            base_url,
            backup_url,
            mime_type,
            codecs: raw.codecs,
            bandwidth: raw.bandwidth,
            width: raw.width,
            height: raw.height,
            codecid: raw.codecid,
        })
    }
}

impl DashStream {
    /// Returns `base_url` followed by all `backup_url` mirrors for CDN failover.
    pub fn all_urls(&self) -> Vec<Url> {
        let mut urls = Vec::with_capacity(1 + self.backup_url.len());
        urls.push(self.base_url.clone());
        urls.extend(self.backup_url.iter().cloned());
        urls
    }

    /// Infer an appropriate container file extension for an audio-only DASH stream.
    pub fn preferred_audio_extension(&self) -> &'static str {
        let mime = self.mime_type.to_ascii_lowercase();
        let codecs = self.codecs.to_ascii_lowercase();
        if mime.contains("flac") || codecs.contains("flac") {
            "flac"
        } else {
            "m4a"
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DolbyAudio {
    #[serde(default, deserialize_with = "null_as_default")]
    pub audio: Vec<DashStream>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FlacAudio {
    #[serde(default)]
    pub display: bool,
    #[serde(default)]
    pub audio: Option<DashStream>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Durl {
    pub url: Url,
    #[serde(default, deserialize_with = "null_as_default")]
    pub backup_url: Vec<Url>,
    #[serde(default)]
    pub size: u64,
}

impl Durl {
    pub fn all_urls(&self) -> Vec<Url> {
        let mut urls = Vec::with_capacity(1 + self.backup_url.len());
        urls.push(self.url.clone());
        urls.extend(self.backup_url.iter().cloned());
        urls
    }
}

fn select_video_stream_capped(
    streams: &[DashStream],
    max_quality: u32,
    preferred_codecid: Option<u32>,
) -> Option<&DashStream> {
    let candidates: Vec<&DashStream> = streams.iter().filter(|s| s.id <= max_quality).collect();
    if !candidates.is_empty() {
        return candidates.into_iter().max_by_key(|s| {
            let codec_match = preferred_codecid.is_some_and(|c| s.codecid == c);
            (s.id, codec_match, s.bandwidth)
        });
    }
    // All available streams exceed the requested cap: pick the lowest quality tier
    // (closest to the requested cap), honoring preferred codec and lowest bandwidth.
    streams.iter().min_by_key(|s| {
        let codec_match = preferred_codecid.is_some_and(|c| s.codecid == c);
        (s.id, !codec_match, s.bandwidth)
    })
}

/// Select a DASH video stream according to [`VideoClarity`] and optional preferred `codecid`.
pub fn select_video_stream(
    streams: &[DashStream],
    clarity: VideoClarity,
    preferred_codecid: Option<u32>,
) -> Option<&DashStream> {
    if streams.is_empty() {
        return None;
    }
    match clarity {
        VideoClarity::Best => streams.iter().max_by_key(|s| {
            let codec_match = preferred_codecid.is_some_and(|c| s.codecid == c);
            (s.id, codec_match, s.bandwidth)
        }),
        VideoClarity::Worst => streams.iter().min_by_key(|s| {
            let codec_match = preferred_codecid.is_some_and(|c| s.codecid == c);
            (s.id, !codec_match, s.bandwidth)
        }),
        other => {
            if let Some(max_q) = other.max_qn() {
                select_video_stream_capped(streams, max_q, preferred_codecid)
            } else {
                streams.iter().max_by_key(|s| {
                    let codec_match = preferred_codecid.is_some_and(|c| s.codecid == c);
                    (s.id, codec_match, s.bandwidth)
                })
            }
        }
    }
}

/// Select the best video stream (highest quality `id` up to `max_quality`, tie-broken by preferred
/// codec then bandwidth; falls back to the lowest `id` if all streams exceed `max_quality`).
pub fn select_best_video(
    streams: &[DashStream],
    max_quality: Option<u32>,
    preferred_codecid: Option<u32>,
) -> Option<&DashStream> {
    match max_quality {
        None => select_video_stream(streams, VideoClarity::Best, preferred_codecid),
        Some(max_q) => select_video_stream_capped(streams, max_q, preferred_codecid),
    }
}

/// Select the highest-bitrate audio stream (checking Hi-Res FLAC, Dolby, and standard DASH audio).
pub fn select_best_audio(dash: &Dash) -> Option<&DashStream> {
    if let Some(flac) = dash.flac.as_ref().and_then(|f| f.audio.as_ref()) {
        return Some(flac);
    }
    if let Some(dolby) = dash
        .dolby
        .as_ref()
        .and_then(|d| d.audio.iter().max_by_key(|s| (s.id, s.bandwidth)))
    {
        return Some(dolby);
    }
    dash.audio.iter().max_by_key(|s| (s.id, s.bandwidth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::BiliEnvelope;

    fn mk_stream(id: u32, codecid: u32, bandwidth: u64) -> DashStream {
        DashStream {
            id,
            base_url: Url::parse(&format!(
                "https://cdn.example/v_{id}_{codecid}_{bandwidth}.m4s"
            ))
            .unwrap(),
            backup_url: vec![],
            mime_type: "video/mp4".to_string(),
            codecs: String::new(),
            bandwidth,
            width: 0,
            height: 0,
            codecid,
        }
    }

    #[test]
    fn dash_deserializes_dual_camel_and_snake_case_keys() {
        // Real Bilibili `/x/player/wbi/playurl` DASH objects include BOTH camelCase and snake_case keys.
        let dual_json = r#"{
            "code": 0,
            "message": "0",
            "msg": "0",
            "data": {
                "quality": 80,
                "dash": {
                    "video": [{
                        "id": 80,
                        "baseUrl": "https://camel-cdn.example/v.m4s",
                        "base_url": "https://snake-cdn.example/v.m4s",
                        "backupUrl": ["https://camel-backup.example/v.m4s"],
                        "backup_url": ["https://snake-backup.example/v.m4s"],
                        "mimeType": "video/mp4",
                        "mime_type": "video/mp4",
                        "codecs": "avc1.640032",
                        "bandwidth": 3000000,
                        "width": 1920,
                        "height": 1080,
                        "frameRate": "30",
                        "frame_rate": "30",
                        "startWithSap": 1,
                        "start_with_sap": 1,
                        "codecid": 7
                    }],
                    "audio": [{
                        "id": 30280,
                        "baseUrl": "https://camel-cdn.example/a.m4s",
                        "base_url": null,
                        "backupUrl": ["https://camel-backup.example/a.m4s"],
                        "backup_url": null,
                        "mimeType": "audio/mp4",
                        "mime_type": "",
                        "codecs": "mp4a.40.2",
                        "bandwidth": 192000,
                        "codecid": 0
                    }]
                },
                "durl": null
            }
        }"#;
        let env: BiliEnvelope<DashData> = serde_json::from_str(dual_json).unwrap();
        let dash = env.data.unwrap().dash.unwrap();

        let video = &dash.video[0];
        assert_eq!(video.id, 80);
        assert_eq!(video.base_url.as_str(), "https://snake-cdn.example/v.m4s");
        assert_eq!(
            video.backup_url.iter().map(Url::as_str).collect::<Vec<_>>(),
            vec!["https://snake-backup.example/v.m4s"]
        );
        assert_eq!(video.mime_type, "video/mp4");
        assert_eq!(video.width, 1920);
        assert_eq!(video.height, 1080);

        let audio = &dash.audio[0];
        assert_eq!(audio.id, 30280);
        assert_eq!(audio.base_url.as_str(), "https://camel-cdn.example/a.m4s");
        assert_eq!(
            audio.backup_url.iter().map(Url::as_str).collect::<Vec<_>>(),
            vec!["https://camel-backup.example/a.m4s"]
        );
        assert_eq!(audio.mime_type, "audio/mp4");

        // camelCase-only stream
        let camel_only = r#"{
            "id": 64,
            "baseUrl": "https://camel.example/v720.m4s",
            "backupUrl": ["https://camel-b1.example/v720.m4s"],
            "mimeType": "video/mp4",
            "codecs": "hev1.1.6.L120.90",
            "bandwidth": 1500000,
            "codecid": 12
        }"#;
        let camel_stream: DashStream = serde_json::from_str(camel_only).unwrap();
        assert_eq!(
            camel_stream.base_url.as_str(),
            "https://camel.example/v720.m4s"
        );
        assert_eq!(camel_stream.backup_url.len(), 1);
        assert_eq!(camel_stream.mime_type, "video/mp4");

        // snake_case-only stream
        let snake_only = r#"{
            "id": 30251,
            "base_url": "https://snake.example/a.flac",
            "backup_url": ["https://snake-b1.example/a.flac"],
            "mime_type": "audio/flac",
            "codecs": "fLaC",
            "bandwidth": 1000000,
            "codecid": 0
        }"#;
        let snake_stream: DashStream = serde_json::from_str(snake_only).unwrap();
        assert_eq!(
            snake_stream.base_url.as_str(),
            "https://snake.example/a.flac"
        );
        assert_eq!(snake_stream.backup_url.len(), 1);
        assert_eq!(snake_stream.mime_type, "audio/flac");
        assert_eq!(snake_stream.preferred_audio_extension(), "flac");

        // Missing both `base_url` and `baseUrl` must fail cleanly
        let missing_base = r#"{"id": 80, "mime_type": "video/mp4"}"#;
        assert!(serde_json::from_str::<DashStream>(missing_base).is_err());
    }

    #[test]
    fn dash_deserializes_null_audio_backup_urls_and_durl() {
        let json = r#"{
            "code": 0,
            "data": {
                "quality": 80,
                "dash": {
                    "video": [{
                        "id": 80,
                        "baseUrl": "https://cdn1.example/v.m4s",
                        "backupUrl": ["https://cdn2.example/v.m4s"],
                        "mimeType": "video/mp4",
                        "codecs": "avc1.640032",
                        "bandwidth": 3000000,
                        "codecid": 7
                    }],
                    "audio": null
                },
                "durl": null
            }
        }"#;
        let env: BiliEnvelope<DashData> = serde_json::from_str(json).unwrap();
        let data = env.data.unwrap();
        let dash = data.dash.unwrap();
        assert_eq!(dash.video.len(), 1);
        assert_eq!(dash.video[0].all_urls().len(), 2);
        assert!(dash.audio.is_empty());
        assert!(data.durl.is_empty());
    }

    #[test]
    fn audio_extension_prefers_m4a_or_flac_not_mp3() {
        let aac = DashStream {
            id: 30280,
            base_url: Url::parse("https://cdn.example/a.m4s").unwrap(),
            backup_url: vec![],
            mime_type: "audio/mp4".to_string(),
            codecs: "mp4a.40.2".to_string(),
            bandwidth: 192_000,
            width: 0,
            height: 0,
            codecid: 0,
        };
        assert_eq!(aac.preferred_audio_extension(), "m4a");

        let flac = DashStream {
            mime_type: "audio/flac".to_string(),
            codecs: "fLaC".to_string(),
            ..aac
        };
        assert_eq!(flac.preferred_audio_extension(), "flac");
    }

    #[test]
    fn select_video_stream_handles_best_worst_capped_and_lowest_fallback() {
        assert!(select_video_stream(&[], VideoClarity::Best, None).is_none());
        assert!(select_video_stream(&[], VideoClarity::Worst, None).is_none());
        assert!(select_video_stream(&[], VideoClarity::P720, None).is_none());

        let streams = vec![
            mk_stream(32, 7, 500_000),
            mk_stream(32, 12, 400_000),
            mk_stream(64, 7, 1_200_000),
            mk_stream(64, 12, 900_000),
            mk_stream(80, 7, 3_000_000),
            mk_stream(80, 12, 2_200_000),
            mk_stream(80, 13, 2_000_000),
        ];

        // Best without codec preference -> highest id, highest bandwidth
        let best = select_video_stream(&streams, VideoClarity::Best, None).unwrap();
        assert_eq!((best.id, best.codecid, best.bandwidth), (80, 7, 3_000_000));

        // Best with AV1 (13) preference -> picks 80 AV1 even with lower bandwidth
        let best_av1 = select_video_stream(&streams, VideoClarity::Best, Some(13)).unwrap();
        assert_eq!((best_av1.id, best_av1.codecid), (80, 13));

        // Worst without codec preference -> lowest id, lowest bandwidth
        let worst = select_video_stream(&streams, VideoClarity::Worst, None).unwrap();
        assert_eq!(
            (worst.id, worst.codecid, worst.bandwidth),
            (32, 12, 400_000)
        );

        // Worst with AVC (7) preference -> lowest id, preferring codec match over lower bandwidth
        let worst_avc = select_video_stream(&streams, VideoClarity::Worst, Some(7)).unwrap();
        assert_eq!((worst_avc.id, worst_avc.codecid), (32, 7));

        // Capped tier exact match (720p = 64) with HEVC (12) preference
        let cap_720 = select_video_stream(&streams, VideoClarity::P720, Some(12)).unwrap();
        assert_eq!((cap_720.id, cap_720.codecid), (64, 12));

        // Capped tier step-down (720p60 = 74 when max <= 74 is 64)
        let cap_720p60 = select_video_stream(&streams, VideoClarity::P720HighFrame, None).unwrap();
        assert_eq!((cap_720p60.id, cap_720p60.bandwidth), (64, 1_200_000));

        // Capped tier where ALL available streams exceed cap (request 240p=6 or 360p=16 when min is 32):
        // MUST fall back to lowest id (32), NOT highest id (80)!
        let cap_low = select_video_stream(&streams, VideoClarity::P360, None).unwrap();
        assert_eq!((cap_low.id, cap_low.codecid), (32, 12));

        // Same fallback honoring preferred codec (AVC = 7 at lowest id 32)
        let cap_low_avc = select_video_stream(&streams, VideoClarity::P360, Some(7)).unwrap();
        assert_eq!((cap_low_avc.id, cap_low_avc.codecid), (32, 7));

        // select_best_video delegates with identical cap & fallback behavior
        let legacy_best = select_best_video(&streams, None, Some(12)).unwrap();
        assert_eq!((legacy_best.id, legacy_best.codecid), (80, 12));
        let legacy_fallback = select_best_video(&streams, Some(16), Some(7)).unwrap();
        assert_eq!((legacy_fallback.id, legacy_fallback.codecid), (32, 7));
    }
}
