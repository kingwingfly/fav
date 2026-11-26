use serde::Deserialize;
use url::Url;

use super::null_as_default;

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
pub struct DashStream {
    #[serde(default)]
    pub id: u32,
    #[serde(alias = "baseUrl")]
    pub base_url: Url,
    #[serde(default, alias = "backupUrl", deserialize_with = "null_as_default")]
    pub backup_url: Vec<Url>,
    #[serde(default, alias = "mimeType")]
    pub mime_type: String,
    #[serde(default)]
    pub codecs: String,
    #[serde(default)]
    pub bandwidth: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub codecid: u32,
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

/// Select the best video stream (highest quality `id`, tie-broken by preferred codec then bandwidth).
pub fn select_best_video(
    streams: &[DashStream],
    max_quality: Option<u32>,
    preferred_codecid: Option<u32>,
) -> Option<&DashStream> {
    let candidates: Vec<&DashStream> = streams
        .iter()
        .filter(|s| max_quality.is_none_or(|max_q| s.id <= max_q))
        .collect();
    let pool: Vec<&DashStream> = if candidates.is_empty() {
        streams.iter().collect()
    } else {
        candidates
    };
    pool.into_iter().max_by_key(|s| {
        let codec_match = preferred_codecid.is_some_and(|c| s.codecid == c);
        (s.id, codec_match, s.bandwidth)
    })
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
}
