use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::models::{AppSettings, CacheStats, ProviderMode};

const KEY_FREESOUND_API: &str = "freesound_api_key";
const KEY_PROVIDER_MODE: &str = "provider_mode";

#[derive(Debug, Clone)]
pub struct CachedSfx {
    pub id: String,
    pub provider: String,
    pub title: String,
    pub author: String,
    pub license: String,
    pub duration: f32,
    pub preview_url: String,
    pub source_url: String,
    pub tags: Vec<String>,
    pub envelope: Vec<f32>,
}

pub struct Database {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl Database {
    pub fn open(base_dir: &Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(base_dir)?;
        let db_path = base_dir.join("sfx_de_babel.db");
        let conn = Connection::open(&db_path)?;

        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sfx_cache (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                title TEXT NOT NULL,
                author TEXT NOT NULL,
                license TEXT NOT NULL,
                duration REAL NOT NULL,
                preview_url TEXT NOT NULL,
                source_url TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                envelope_json TEXT NOT NULL,
                cached_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sfx_cache_cached_at ON sfx_cache(cached_at);
            ",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
        })
    }

    pub fn get_settings(&self) -> Result<AppSettings, AppError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AppError::InvalidInput("Ошибка блокировки БД".to_string()))?;

        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |row| {
            let k: String = row.get(0)?;
            let v: String = row.get(1)?;
            Ok((k, v))
        })?;

        let mut settings = AppSettings::default();
        for row in rows {
            let (k, v) = row?;
            match k.as_str() {
                KEY_FREESOUND_API => settings.freesound_api_key = v,
                KEY_PROVIDER_MODE => {
                    settings.provider_mode = match v.as_str() {
                        "freesound" => ProviderMode::Freesound,
                        "mixkit" => ProviderMode::Mixkit,
                        _ => ProviderMode::Auto,
                    };
                }
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), AppError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AppError::InvalidInput("Ошибка блокировки БД".to_string()))?;

        let tx = conn.transaction()?;
        let mode_str = match settings.provider_mode {
            ProviderMode::Auto => "auto",
            ProviderMode::Freesound => "freesound",
            ProviderMode::Mixkit => "mixkit",
        };
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY_FREESOUND_API, settings.freesound_api_key.trim()],
        )?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY_PROVIDER_MODE, mode_str],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_cached_batch(&self, ids: &[String]) -> Result<HashMap<String, CachedSfx>, AppError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let conn = self
            .conn
            .lock()
            .map_err(|_| AppError::InvalidInput("Ошибка блокировки БД".to_string()))?;

        let mut result = HashMap::new();
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, provider, title, author, license, duration, preview_url, source_url, tags_json, envelope_json
             FROM sfx_cache WHERE id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)? as f32,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })?;

        for row in rows {
            let (
                id,
                provider,
                title,
                author,
                license,
                duration,
                preview_url,
                source_url,
                tags_json,
                envelope_json,
            ) = row?;

            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let envelope: Vec<f32> = serde_json::from_str(&envelope_json).unwrap_or_default();
            if envelope.is_empty() {
                continue;
            }
            result.insert(
                id.clone(),
                CachedSfx {
                    id,
                    provider,
                    title,
                    author,
                    license,
                    duration,
                    preview_url,
                    source_url,
                    tags,
                    envelope,
                },
            );
        }

        Ok(result)
    }

    pub fn upsert_cached(&self, items: &[CachedSfx]) -> Result<(), AppError> {
        if items.is_empty() {
            return Ok(());
        }

        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AppError::InvalidInput("Ошибка блокировки БД".to_string()))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO sfx_cache (
                    id, provider, title, author, license, duration,
                    preview_url, source_url, tags_json, envelope_json, cached_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(id) DO UPDATE SET
                    provider = excluded.provider,
                    title = excluded.title,
                    author = excluded.author,
                    license = excluded.license,
                    duration = excluded.duration,
                    preview_url = excluded.preview_url,
                    source_url = excluded.source_url,
                    tags_json = excluded.tags_json,
                    envelope_json = excluded.envelope_json,
                    cached_at = excluded.cached_at",
            )?;

            for item in items {
                let tags_json = serde_json::to_string(&item.tags)?;
                let envelope_json = serde_json::to_string(&item.envelope)?;
                stmt.execute(params![
                    item.id,
                    item.provider,
                    item.title,
                    item.author,
                    item.license,
                    item.duration as f64,
                    item.preview_url,
                    item.source_url,
                    tags_json,
                    envelope_json,
                    now,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn clear_cache(&self, temp_wav_dir: &Path) -> Result<usize, AppError> {
        let deleted_rows = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| AppError::InvalidInput("Ошибка блокировки БД".to_string()))?;
            conn.execute("DELETE FROM sfx_cache", [])?
        };

        if temp_wav_dir.exists() {
            for entry in std::fs::read_dir(temp_wav_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file()
                    && path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
                {
                    let _ = std::fs::remove_file(path);
                }
            }
        }

        Ok(deleted_rows)
    }

    pub fn cache_stats(&self, temp_wav_dir: &Path) -> Result<CacheStats, AppError> {
        let entry_count = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| AppError::InvalidInput("Ошибка блокировки БД".to_string()))?;
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM sfx_cache", [], |row| row.get(0))?;
            count.max(0) as usize
        };

        let mut temp_wav_count = 0usize;
        if temp_wav_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(temp_wav_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
                    {
                        temp_wav_count += 1;
                    }
                }
            }
        }

        let db_size_bytes = std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(CacheStats {
            entry_count,
            temp_wav_count,
            db_size_bytes,
        })
    }
}
