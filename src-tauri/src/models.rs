use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ProviderMode {
    #[default]
    Auto,
    Freesound,
    Mixkit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    #[serde(rename = "en")]
    En,
    #[default]
    #[serde(rename = "ru")]
    Ru,
    #[serde(rename = "es")]
    Es,
    #[serde(rename = "pt-BR")]
    PtBr,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub freesound_api_key: String,
    pub provider_mode: ProviderMode,
    #[serde(default)]
    pub language: Language,
    /// Keep only sounds positively known to be usable commercially without attribution.
    #[serde(default)]
    pub commercial_only_no_attribution: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            freesound_api_key: String::new(),
            provider_mode: ProviderMode::Auto,
            language: Language::Ru,
            commercial_only_no_attribution: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub entry_count: usize,
    pub temp_wav_count: usize,
    pub db_size_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub min_duration: f32,
    pub max_duration: f32,
    pub envelope: Option<Vec<f32>>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SfxItem {
    pub id: String,
    pub provider: String,
    pub title: String,
    pub author: String,
    /// License id (`cc0`, `cc-by`, `mixkit-sfx-free`, `unknown`, `local`, ...).
    pub license: String,
    pub license_name: String,
    pub license_url: String,
    pub requires_attribution: bool,
    pub commercial_use_allowed: Option<bool>,
    pub duration: f32,
    pub preview_url: String,
    pub source_url: String,
    pub tags: Vec<String>,
    pub envelope: Vec<f32>,
    pub match_score: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub items: Vec<SfxItem>,
    pub provider_used: String,
    pub warning: Option<String>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatus {
    pub is_playing: bool,
    pub sound_id: Option<String>,
    pub position_secs: f32,
    pub duration_secs: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FxWaveformResult {
    pub envelope: Vec<f32>,
    pub duration_secs: f32,
}
