use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinSet;

use crate::audio::decoder::{decode_audio_bytes, DecodedAudio};
use crate::audio::envelope::{
    compute_rms_envelope, compute_similarity, normalize_envelope, process_batch_parallel,
};
use crate::audio::export::{export_temp_dir, export_to_wav, export_to_wav_24bit, ExportMeta};
use crate::audio::fx::{AudioBuffer, FxChain, FxParams};
use crate::audio::player::AudioPlayer;
use crate::db::{CachedSfx, Database};
use crate::error::AppError;
use crate::license::LicenseInfo;
use crate::models::{
    AppSettings, CacheStats, FxWaveformResult, PlaybackStatus, ProviderMode, SearchRequest,
    SearchResponse, SfxItem,
};
use crate::providers::freesound::FreesoundProvider;
use crate::providers::freesound_web::FreesoundWebProvider;
use crate::providers::mixkit::MixkitProvider;
use crate::providers::{
    normalize_sfx_query, ProviderPage, ProviderSearchParams, RawSfxCandidate, SfxProvider,
};

const MAX_RAM_AUDIO_ENTRIES: usize = 128;
const MAX_CONCURRENT_DOWNLOADS: usize = 10;
const DEFAULT_PAGE_SIZE: usize = 28;

#[derive(Debug, Clone, Copy)]
enum SearchCursor {
    FsApi(u32),
    Mixkit(u32),
    FsWeb(u32),
}

impl SearchCursor {
    fn parse(raw: &str) -> Option<Self> {
        let (kind, page_str) = raw.split_once(':')?;
        let page = page_str.parse::<u32>().ok().filter(|&p| p >= 1)?;
        match kind {
            "fs_api" => Some(Self::FsApi(page)),
            "mixkit" => Some(Self::Mixkit(page)),
            "fs_web" => Some(Self::FsWeb(page)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct SoundMeta {
    title: String,
    preview_url: String,
    author: String,
    license: LicenseInfo,
    source_url: String,
}

impl SoundMeta {
    fn export_meta(&self) -> ExportMeta {
        ExportMeta {
            title: self.title.clone(),
            author: self.author.clone(),
            license_name: self.license.name.clone(),
            license_url: self.license.url.clone(),
            source_url: self.source_url.clone(),
        }
    }
}

struct AudioRamCache {
    map: HashMap<String, Arc<DecodedAudio>>,
    order: VecDeque<String>,
}

impl AudioRamCache {
    fn new() -> Self {
        Self {
            map: HashMap::with_capacity(MAX_RAM_AUDIO_ENTRIES),
            order: VecDeque::with_capacity(MAX_RAM_AUDIO_ENTRIES),
        }
    }

    fn touch_lru(&mut self, id: &str) -> Option<Arc<DecodedAudio>> {
        let item = self.map.get(id)?.clone();
        if let Some(pos) = self.order.iter().position(|k| k == id) {
            self.order.remove(pos);
        }
        self.order.push_back(id.to_string());
        Some(item)
    }

    fn insert(&mut self, id: String, audio: Arc<DecodedAudio>) {
        if self.map.contains_key(&id) {
            if let Some(pos) = self.order.iter().position(|k| k == &id) {
                self.order.remove(pos);
            }
            self.order.push_back(id.clone());
            self.map.insert(id, audio);
            return;
        }
        while self.order.len() >= MAX_RAM_AUDIO_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(id.clone());
        self.map.insert(id, audio);
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

pub struct AppState {
    http: reqwest::Client,
    db: Arc<Database>,
    player: Arc<AudioPlayer>,
    audio_cache: Arc<RwLock<AudioRamCache>>,
    sound_meta: Arc<RwLock<HashMap<String, SoundMeta>>>,
}

impl AppState {
    pub fn new(data_dir: &Path) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .user_agent("SFX-de-Babel/0.1 (Mozilla/5.0)")
            .timeout(Duration::from_secs(16))
            .connect_timeout(Duration::from_secs(8))
            .build()?;

        let db = Arc::new(Database::open(data_dir)?);
        let player = Arc::new(AudioPlayer::new());
        let audio_cache = Arc::new(RwLock::new(AudioRamCache::new()));
        let sound_meta = Arc::new(RwLock::new(HashMap::new()));

        Ok(Self {
            http,
            db,
            player,
            audio_cache,
            sound_meta,
        })
    }

    async fn ensure_decoded(
        &self,
        sound_id: &str,
        preview_url: &str,
    ) -> Result<Arc<DecodedAudio>, AppError> {
        {
            let mut guard = self.audio_cache.write().await;
            if let Some(cached) = guard.touch_lru(sound_id) {
                return Ok(cached);
            }
        }

        let resp = self.http.get(preview_url).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Provider(format!(
                "Не удалось загрузить аудио ({})",
                resp.status()
            )));
        }
        let bytes = resp.bytes().await?;
        let ext_hint = infer_audio_ext(preview_url);

        let decoded = tokio::task::spawn_blocking(move || {
            decode_audio_bytes(bytes, ext_hint.as_deref())
        })
        .await
        .map_err(|e| AppError::AudioDecode(format!("Внутренняя ошибка потока: {e}")))??;

        let arc = Arc::new(decoded);
        let mut guard = self.audio_cache.write().await;
        guard.insert(sound_id.to_string(), Arc::clone(&arc));
        Ok(arc)
    }

    async fn resolve_audio_and_title(
        &self,
        sound_id: &str,
    ) -> Result<(Arc<DecodedAudio>, SoundMeta), AppError> {
        let meta_from_ram = {
            let guard = self.sound_meta.read().await;
            guard.get(sound_id).cloned()
        };

        let meta = if let Some(m) = meta_from_ram {
            m
        } else {
            let db_map = self.db.get_cached_batch(&[sound_id.to_string()])?;
            if let Some(cached) = db_map.get(sound_id) {
                let m = SoundMeta {
                    title: cached.title.clone(),
                    preview_url: cached.preview_url.clone(),
                    author: cached.author.clone(),
                    license: cached.license.clone(),
                    source_url: cached.source_url.clone(),
                };
                let mut guard = self.sound_meta.write().await;
                guard.insert(sound_id.to_string(), m.clone());
                m
            } else {
                // If audio is already in LRU cache without metadata, fall back to sound_id as title.
                let mut ram_guard = self.audio_cache.write().await;
                if let Some(audio) = ram_guard.touch_lru(sound_id) {
                    return Ok((
                        audio,
                        SoundMeta {
                            title: sound_id.to_string(),
                            preview_url: String::new(),
                            author: String::new(),
                            license: LicenseInfo::unknown(),
                            source_url: String::new(),
                        },
                    ));
                }
                return Err(AppError::InvalidInput(format!(
                    "Звук {sound_id} не найден. Выполните поиск заново."
                )));
            }
        };

        let audio = self.ensure_decoded(sound_id, &meta.preview_url).await?;
        Ok((audio, meta))
    }
}

fn infer_audio_ext(url: &str) -> Option<String> {
    let path_part = url.split('?').next().unwrap_or(url);
    let ext = path_part.rsplit('.').next()?.trim().to_ascii_lowercase();
    match ext.as_str() {
        "mp3" | "ogg" | "oga" | "wav" | "flac" => Some(ext),
        _ => None,
    }
}

async fn download_candidates(
    client: &reqwest::Client,
    candidates: &[&RawSfxCandidate],
) -> Vec<(String, Bytes, Option<String>)> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DOWNLOADS));
    let mut join_set = JoinSet::new();

    for cand in candidates {
        let client = client.clone();
        let sem = Arc::clone(&semaphore);
        let id = cand.id.clone();
        let url = cand.preview_url.clone();
        let ext_hint = infer_audio_ext(&url);

        join_set.spawn(async move {
            let Ok(_permit) = sem.acquire().await else {
                return None;
            };
            let resp = client.get(&url).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let bytes = resp.bytes().await.ok()?;
            if bytes.is_empty() {
                return None;
            }
            Some((id, bytes, ext_hint))
        });
    }

    let mut out = Vec::new();
    while let Some(res) = join_set.join_next().await {
        if let Ok(Some(item)) = res {
            out.push(item);
        }
    }
    out
}

#[tauri::command]
pub async fn search_sfx(
    state: tauri::State<'_, AppState>,
    request: SearchRequest,
) -> Result<SearchResponse, String> {
    let min_duration = request.min_duration.clamp(0.05, 60.0);
    let max_duration = request.max_duration.clamp(min_duration + 0.05, 120.0);
    let is_append = request.cursor.is_some();

    let settings = state.db.get_settings().map_err(String::from)?;
    let normalized_query = normalize_sfx_query(&request.query);

    let mut warning: Option<String> = None;
    let resolved_cursor: SearchCursor;
    let page_result: ProviderPage;
    let provider_used: String;

    if let Some(raw_cursor) = request.cursor.as_deref() {
        resolved_cursor = SearchCursor::parse(raw_cursor)
            .ok_or_else(|| format!("Некорректный курсор пагинации: {raw_cursor}"))?;
        let page_num = match resolved_cursor {
            SearchCursor::FsApi(p) | SearchCursor::Mixkit(p) | SearchCursor::FsWeb(p) => p,
        };
        let search_params = ProviderSearchParams {
            query: normalized_query.clone(),
            min_duration,
            max_duration,
            page: page_num,
            page_size: DEFAULT_PAGE_SIZE,
            cc0_only: settings.commercial_only_no_attribution,
        };

        match resolved_cursor {
            SearchCursor::FsApi(_) => {
                if settings.freesound_api_key.trim().is_empty() {
                    return Err(AppError::MissingApiKey.into());
                }
                let fs = FreesoundProvider::new(settings.freesound_api_key.clone());
                page_result = fs
                    .search(&state.http, &search_params)
                    .await
                    .map_err(String::from)?;
                provider_used = fs.name().to_string();
            }
            SearchCursor::Mixkit(_) => {
                let mk = MixkitProvider::new();
                page_result = mk
                    .search(&state.http, &search_params)
                    .await
                    .map_err(String::from)?;
                provider_used = mk.name().to_string();
            }
            SearchCursor::FsWeb(n) => {
                let fsw = FreesoundWebProvider::new();
                match fsw.search(&state.http, &search_params).await {
                    Ok(res) => {
                        page_result = res;
                        provider_used = fsw.name().to_string();
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        if msg.contains("429") {
                            return Ok(SearchResponse {
                                items: Vec::new(),
                                provider_used: fsw.name().to_string(),
                                warning: Some(msg),
                                next_cursor: Some(format!("fs_web:{n}")),
                            });
                        }
                        return Err(msg);
                    }
                }
            }
        }
    } else {
        let search_params = ProviderSearchParams {
            query: normalized_query.clone(),
            min_duration,
            max_duration,
            page: 1,
            page_size: DEFAULT_PAGE_SIZE,
            cc0_only: settings.commercial_only_no_attribution,
        };

        match settings.provider_mode {
            ProviderMode::Freesound => {
                if settings.freesound_api_key.trim().is_empty() {
                    return Err(AppError::MissingApiKey.into());
                }
                let fs = FreesoundProvider::new(settings.freesound_api_key.clone());
                page_result = fs
                    .search(&state.http, &search_params)
                    .await
                    .map_err(String::from)?;
                provider_used = fs.name().to_string();
                resolved_cursor = SearchCursor::FsApi(1);
            }
            ProviderMode::Mixkit => {
                let mk = MixkitProvider::new();
                page_result = mk
                    .search(&state.http, &search_params)
                    .await
                    .map_err(String::from)?;
                provider_used = mk.name().to_string();
                resolved_cursor = SearchCursor::Mixkit(1);
            }
            ProviderMode::Auto => {
                if !settings.freesound_api_key.trim().is_empty() {
                    let fs = FreesoundProvider::new(settings.freesound_api_key.clone());
                    match fs.search(&state.http, &search_params).await {
                        Ok(res) => {
                            page_result = res;
                            provider_used = fs.name().to_string();
                            resolved_cursor = SearchCursor::FsApi(1);
                        }
                        Err(err) => {
                            let msg = err.to_string();
                            if msg.contains("401") || msg.contains("403") {
                                return Err(msg);
                            }
                            warning = Some(format!(
                                "Freesound API недоступен ({msg}). Использован резервный источник."
                            ));
                            let mk = MixkitProvider::new();
                            let mk_res = mk
                                .search(&state.http, &search_params)
                                .await
                                .map_err(String::from)?;
                            if !mk_res.candidates.is_empty() {
                                page_result = mk_res;
                                provider_used = mk.name().to_string();
                                resolved_cursor = SearchCursor::Mixkit(1);
                            } else {
                                let fsw = FreesoundWebProvider::new();
                                page_result = fsw
                                    .search(&state.http, &search_params)
                                    .await
                                    .map_err(String::from)?;
                                provider_used = fsw.name().to_string();
                                resolved_cursor = SearchCursor::FsWeb(1);
                            }
                        }
                    }
                } else {
                    let mk = MixkitProvider::new();
                    let mk_res = mk.search(&state.http, &search_params).await;
                    match mk_res {
                        Ok(res) if !res.candidates.is_empty() => {
                            page_result = res;
                            provider_used = mk.name().to_string();
                            resolved_cursor = SearchCursor::Mixkit(1);
                        }
                        _ => {
                            let fsw = FreesoundWebProvider::new();
                            page_result = fsw
                                .search(&state.http, &search_params)
                                .await
                                .map_err(String::from)?;
                            provider_used = fsw.name().to_string();
                            resolved_cursor = SearchCursor::FsWeb(1);
                        }
                    }
                }
            }
        }
    }

    let next_cursor: Option<String> = match resolved_cursor {
        SearchCursor::FsApi(n) => page_result
            .has_next
            .then(|| format!("fs_api:{}", n + 1)),
        SearchCursor::Mixkit(n) => {
            if page_result.has_next {
                Some(format!("mixkit:{}", n + 1))
            } else if settings.provider_mode == ProviderMode::Auto {
                Some("fs_web:1".to_string())
            } else {
                None
            }
        }
        SearchCursor::FsWeb(n) => page_result
            .has_next
            .then(|| format!("fs_web:{}", n + 1)),
    };

    let mut candidates = page_result.candidates;
    if settings.commercial_only_no_attribution {
        // Keep only sounds positively known to be usable commercially without credit.
        // Unknown licenses are excluded: we never guess.
        candidates.retain(|c| c.license.is_commercial_no_attribution());
    }
    if candidates.is_empty() {
        return Ok(SearchResponse {
            items: Vec::new(),
            provider_used,
            warning,
            next_cursor,
        });
    }

    let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let cached_map = state.db.get_cached_batch(&ids).map_err(String::from)?;

    let missing_candidates: Vec<&RawSfxCandidate> = candidates
        .iter()
        .filter(|c| !cached_map.contains_key(&c.id))
        .collect();

    let mut newly_processed: HashMap<String, (f32, Vec<f32>)> = HashMap::new();

    if !missing_candidates.is_empty() {
        let downloaded = download_candidates(&state.http, &missing_candidates).await;
        let decoded_batch = tokio::task::spawn_blocking(move || process_batch_parallel(downloaded))
            .await
            .map_err(|e| format!("Ошибка пула анализа аудио: {e}"))?;

        let mut to_cache_db = Vec::new();
        let candidate_lookup: HashMap<&str, &RawSfxCandidate> =
            candidates.iter().map(|c| (c.id.as_str(), c)).collect();

        {
            let mut ram_guard = state.audio_cache.write().await;
            for (id, result) in decoded_batch {
                if let Ok((decoded, env)) = result {
                    let exact_dur = decoded.duration_secs;
                    ram_guard.insert(id.clone(), Arc::new(decoded));
                    newly_processed.insert(id.clone(), (exact_dur, env.clone()));

                    if let Some(cand) = candidate_lookup.get(id.as_str()) {
                        to_cache_db.push(CachedSfx {
                            id,
                            provider: cand.provider.clone(),
                            title: cand.title.clone(),
                            author: cand.author.clone(),
                            license: cand.license.clone(),
                            duration: exact_dur,
                            preview_url: cand.preview_url.clone(),
                            source_url: cand.source_url.clone(),
                            tags: cand.tags.clone(),
                            envelope: env,
                        });
                    }
                }
            }
        }

        if let Err(err) = state.db.upsert_cached(&to_cache_db) {
            warning.get_or_insert_with(|| format!("Кэш SQLite временно недоступен: {err}"));
        }
    }

    let target_envelope = request.envelope.and_then(|raw| {
        let norm = normalize_envelope(&raw);
        if norm.iter().any(|&v| v > 1e-4) {
            Some(norm)
        } else {
            None
        }
    });

    let mut all_items = Vec::with_capacity(candidates.len());
    {
        let mut meta_guard = state.sound_meta.write().await;
        for cand in candidates {
            let (duration, envelope) = if let Some(cached) = cached_map.get(&cand.id) {
                (cached.duration, cached.envelope.clone())
            } else if let Some((dur, env)) = newly_processed.get(&cand.id) {
                (*dur, env.clone())
            } else {
                continue;
            };

            meta_guard.insert(
                cand.id.clone(),
                SoundMeta {
                    title: cand.title.clone(),
                    preview_url: cand.preview_url.clone(),
                    author: cand.author.clone(),
                    license: cand.license.clone(),
                    source_url: cand.source_url.clone(),
                },
            );

            let match_score = target_envelope
                .as_ref()
                .map(|target| compute_similarity(target, &envelope));

            all_items.push(SfxItem {
                id: cand.id,
                provider: cand.provider,
                title: cand.title,
                author: cand.author,
                license: cand.license.id,
                license_name: cand.license.name,
                license_url: cand.license.url,
                requires_attribution: cand.license.requires_attribution,
                commercial_use_allowed: cand.license.commercial_use_allowed,
                duration,
                preview_url: cand.preview_url,
                source_url: cand.source_url,
                tags: cand.tags,
                envelope,
                match_score,
            });
        }
    }

    let mut strict_items: Vec<SfxItem> = all_items
        .iter()
        .filter(|item| item.duration >= min_duration && item.duration <= max_duration)
        .cloned()
        .collect();

    if !is_append && strict_items.is_empty() && !all_items.is_empty() {
        warning.get_or_insert_with(|| {
            format!(
                "В диапазоне {min_duration:.1}–{max_duration:.1} с точных совпадений не найдено; показаны ближайшие по длительности."
            )
        });
        strict_items = all_items;
    }

    if target_envelope.is_some() {
        strict_items.sort_by(|a, b| {
            let sb = b.match_score.unwrap_or(0.0);
            let sa = a.match_score.unwrap_or(0.0);
            sb.total_cmp(&sa)
        });
    }

    Ok(SearchResponse {
        items: strict_items,
        provider_used,
        warning,
        next_cursor,
    })
}

#[tauri::command]
pub async fn play_sfx(
    state: tauri::State<'_, AppState>,
    sound_id: String,
    preview_url: String,
    start_secs: Option<f32>,
    volume: Option<f32>,
) -> Result<PlaybackStatus, String> {
    let audio = state
        .ensure_decoded(&sound_id, &preview_url)
        .await
        .map_err(String::from)?;

    state
        .player
        .play(sound_id, &audio, start_secs, volume)
        .map_err(String::from)
}

#[tauri::command]
pub async fn preview_sfx_with_fx(
    state: tauri::State<'_, AppState>,
    sound_id: String,
    fx: FxParams,
    start_secs: Option<f32>,
    volume: Option<f32>,
) -> Result<Vec<f32>, String> {
    let (original, _) = state
        .resolve_audio_and_title(&sound_id)
        .await
        .map_err(String::from)?;

    let (processed, envelope) = tokio::task::spawn_blocking(move || {
        let raw_buf = AudioBuffer::from_decoded(&original);
        let fx_buf = FxChain::process(&raw_buf, &fx);
        let decoded = fx_buf.to_decoded();
        let env = compute_rms_envelope(&decoded.mono_samples);
        (decoded, env)
    })
    .await
    .map_err(|e| format!("Ошибка потока DSP: {e}"))?;

    state
        .player
        .play(sound_id, &processed, start_secs, volume)
        .map_err(String::from)?;

    Ok(envelope)
}

#[tauri::command]
pub async fn render_fx_waveform(
    state: tauri::State<'_, AppState>,
    sound_id: String,
    fx: FxParams,
) -> Result<FxWaveformResult, String> {
    let (original, _) = state
        .resolve_audio_and_title(&sound_id)
        .await
        .map_err(String::from)?;

    let result = tokio::task::spawn_blocking(move || {
        let raw_buf = AudioBuffer::from_decoded(&original);
        let fx_buf = FxChain::process(&raw_buf, &fx);
        let decoded = fx_buf.to_decoded();
        let envelope = compute_rms_envelope(&decoded.mono_samples);
        FxWaveformResult {
            envelope,
            duration_secs: decoded.duration_secs,
        }
    })
    .await
    .map_err(|e| format!("Ошибка расчёта вейвформы DSP: {e}"))?;

    Ok(result)
}

#[tauri::command]
pub fn stop_sfx(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.player.stop().map_err(String::from)
}

#[tauri::command]
pub fn set_volume(state: tauri::State<'_, AppState>, volume: f32) -> Result<(), String> {
    state.player.set_volume(volume).map_err(String::from)
}

#[tauri::command]
pub fn get_playback_status(state: tauri::State<'_, AppState>) -> Result<PlaybackStatus, String> {
    Ok(state.player.get_status())
}

#[tauri::command]
pub async fn prepare_drag_wav(
    state: tauri::State<'_, AppState>,
    sound_id: String,
    title: String,
    preview_url: String,
) -> Result<String, String> {
    {
        let mut guard = state.sound_meta.write().await;
        guard.insert(
            sound_id.clone(),
            SoundMeta {
                title: title.clone(),
                preview_url: preview_url.clone(),
                author: String::new(),
                license: LicenseInfo::unknown(),
                source_url: String::new(),
            },
        );
    }

    let audio = state
        .ensure_decoded(&sound_id, &preview_url)
        .await
        .map_err(String::from)?;

    let path = tokio::task::spawn_blocking(move || export_to_wav(&sound_id, &title, &audio, None))
        .await
        .map_err(|e| format!("Ошибка потока экспорта WAV: {e}"))?
        .map_err(String::from)?;

    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn prepare_drag_file(
    state: tauri::State<'_, AppState>,
    sound_id: String,
    fx: FxParams,
) -> Result<String, String> {
    let (original, meta) = state
        .resolve_audio_and_title(&sound_id)
        .await
        .map_err(String::from)?;

    let path = tokio::task::spawn_blocking(move || {
        let raw_buf = AudioBuffer::from_decoded(&original);
        let fx_buf = FxChain::process(&raw_buf, &fx);
        let processed = fx_buf.to_decoded();
        let suffix = fx.filename_suffix();
        let export_meta = meta.export_meta();
        export_to_wav_24bit(&sound_id, &meta.title, &suffix, &processed, Some(&export_meta))
    })
    .await
    .map_err(|e| format!("Ошибка потока экспорта 24-bit WAV: {e}"))?
    .map_err(String::from)?;

    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    state.db.get_settings().map_err(String::from)
}

#[tauri::command]
pub fn save_settings(
    state: tauri::State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    state.db.save_settings(&settings).map_err(String::from)
}

#[tauri::command]
pub fn get_cache_stats(state: tauri::State<'_, AppState>) -> Result<CacheStats, String> {
    let dir = export_temp_dir();
    state.db.cache_stats(&dir).map_err(String::from)
}

#[tauri::command]
pub async fn clear_cache(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    {
        let mut guard = state.audio_cache.write().await;
        guard.clear();
    }
    let dir = export_temp_dir();
    state.db.clear_cache(&dir).map_err(String::from)
}
