//! Local library: recursive scan, incremental parallel indexing (64-point RMS envelopes)
//! and envelope-based ranking over the whole indexed folder.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use bytes::Bytes;
use rayon::prelude::*;
use serde::Serialize;

use crate::audio::decoder::{decode_audio_bytes, DecodedAudio};
use crate::audio::envelope::{compute_rms_envelope, compute_similarity_weighted};
use crate::db::{Database, LibraryRow};
use crate::error::AppError;

pub const LIB_ID_PREFIX: &str = "lib_";
const AUDIO_EXTS: [&str; 4] = ["mp3", "ogg", "wav", "flac"];
/// Files above this size are skipped: the whole file is decoded into RAM.
const MAX_FILE_BYTES: u64 = 150 * 1024 * 1024;
/// Files analysed per parallel batch (also the progress/cancel granularity).
const CHUNK: usize = 48;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    /// `scanning` | `indexing` | `done` | `cancelled`
    pub phase: &'static str,
    pub total: usize,
    pub processed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    /// Audio files found on disk.
    pub total: usize,
    /// New or changed files analysed in this run.
    pub indexed: usize,
    /// Files skipped because (path, mtime, size) did not change.
    pub unchanged: usize,
    pub failed: usize,
    /// Index rows dropped because the file no longer exists.
    pub removed: usize,
    pub cancelled: bool,
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: String,
    pub mtime_ms: i64,
    pub size: i64,
}

/// Normalizes a root directory into a prefix that cannot match sibling folders
/// (`/music` must not match `/music2`).
pub fn root_prefix(root: &str) -> String {
    let trimmed = root.trim().trim_end_matches(['/', '\\']);
    format!("{trimmed}{}", std::path::MAIN_SEPARATOR)
}

/// Stable id for a library file (FNV-1a 64 of its path).
pub fn library_item_id(path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{LIB_ID_PREFIX}{hash:016x}")
}

pub fn file_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sound".to_string())
}

fn has_audio_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Recursively lists mp3/ogg/wav/flac files. Symlinked directories are not followed
/// (no cycles); non-UTF-8 paths are skipped. Stops early when `cancel` is set.
pub fn scan_audio_files(root: &Path, cancel: &AtomicBool) -> Vec<ScannedFile> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() && has_audio_ext(&path) {
                let Some(path_str) = path.to_str() else { continue };
                let Ok(meta) = entry.metadata() else { continue };
                let mtime_ms = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                out.push(ScannedFile {
                    path: path_str.to_string(),
                    mtime_ms,
                    size: meta.len() as i64,
                });
            }
        }
    }
    out
}

/// Reads and decodes one local file (size-capped).
pub fn decode_file(path: &str) -> Result<DecodedAudio, AppError> {
    let p = Path::new(path);
    let size = std::fs::metadata(p)?.len();
    if size > MAX_FILE_BYTES {
        return Err(AppError::AudioDecode(format!(
            "Файл слишком большой ({} МБ)",
            size / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(p)?;
    let ext = p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    decode_audio_bytes(Bytes::from(bytes), ext.as_deref())
}

fn analyze(file: &ScannedFile) -> Result<LibraryRow, AppError> {
    let decoded = decode_file(&file.path)?;
    Ok(LibraryRow {
        path: file.path.clone(),
        mtime_ms: file.mtime_ms,
        size: file.size,
        duration: decoded.duration_secs,
        envelope: compute_rms_envelope(&decoded.mono_samples),
    })
}

/// Indexes `root`, analysing only files whose `(path, mtime, size)` changed.
/// `on_progress` is called from the calling thread between batches.
pub fn index_library(
    db: &Database,
    root: &str,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(IndexProgress),
) -> Result<IndexSummary, AppError> {
    let root_path = Path::new(root.trim());
    if !root_path.is_dir() {
        return Err(AppError::InvalidInput(format!(
            "Папка не найдена: {}",
            root.trim()
        )));
    }
    let prefix = root_prefix(root);

    on_progress(IndexProgress {
        phase: "scanning",
        total: 0,
        processed: 0,
        failed: 0,
    });
    let files = scan_audio_files(root_path, cancel);
    let mut summary = IndexSummary {
        total: files.len(),
        ..Default::default()
    };
    if cancel.load(Ordering::Relaxed) {
        summary.cancelled = true;
        on_progress(IndexProgress {
            phase: "cancelled",
            total: summary.total,
            processed: 0,
            failed: 0,
        });
        return Ok(summary);
    }

    let existing = db.library_existing(&prefix)?;
    let on_disk: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let removed: Vec<String> = existing
        .keys()
        .filter(|p| !on_disk.contains(p.as_str()))
        .cloned()
        .collect();
    summary.removed = removed.len();
    db.library_delete(&removed)?;

    let todo: Vec<&ScannedFile> = files
        .iter()
        .filter(|f| existing.get(&f.path) != Some(&(f.mtime_ms, f.size)))
        .collect();
    summary.unchanged = files.len() - todo.len();

    let mut processed = 0usize;
    for chunk in todo.chunks(CHUNK) {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        // `None` = skipped because cancellation arrived mid-batch.
        let results: Vec<Option<Result<LibraryRow, AppError>>> = chunk
            .par_iter()
            .map(|f| {
                if cancel.load(Ordering::Relaxed) {
                    None
                } else {
                    Some(analyze(f))
                }
            })
            .collect();

        let mut ok_rows = Vec::with_capacity(results.len());
        for r in results.into_iter().flatten() {
            processed += 1;
            match r {
                Ok(row) => ok_rows.push(row),
                Err(_) => summary.failed += 1,
            }
        }
        summary.indexed += ok_rows.len();
        db.library_upsert(&ok_rows)?;

        on_progress(IndexProgress {
            phase: "indexing",
            total: todo.len(),
            processed,
            failed: summary.failed,
        });
    }
    if cancel.load(Ordering::Relaxed) {
        summary.cancelled = true;
    }

    on_progress(IndexProgress {
        phase: if summary.cancelled { "cancelled" } else { "done" },
        total: todo.len(),
        processed,
        failed: summary.failed,
    });
    Ok(summary)
}

#[derive(Debug, Clone)]
pub struct LibraryHit {
    pub row: LibraryRow,
    pub score: Option<f32>,
}

/// Applies the duration range. When nothing is inside it, returns everything and `true`
/// so the caller can tell the user the range was relaxed.
pub fn filter_by_duration(rows: Vec<LibraryRow>, min: f32, max: f32) -> (Vec<LibraryRow>, bool) {
    let inside: Vec<LibraryRow> = rows
        .iter()
        .filter(|r| r.duration >= min && r.duration <= max)
        .cloned()
        .collect();
    if inside.is_empty() && !rows.is_empty() {
        (rows, true)
    } else {
        (inside, false)
    }
}

/// Ranks the whole library.
///
/// * `tokens` (lowercase) must all appear in the file path;
/// * with a `target` envelope: sorted by similarity (cosine + RMSE blend, see `cosine_weight`);
/// * without: sorted by path.
pub fn rank_library(
    rows: Vec<LibraryRow>,
    tokens: &[String],
    target: Option<&[f32]>,
    cosine_weight: f32,
) -> Vec<LibraryHit> {
    let mut hits: Vec<LibraryHit> = rows
        .into_par_iter()
        .filter(|r| {
            if tokens.is_empty() {
                return true;
            }
            let lower = r.path.to_lowercase();
            tokens.iter().all(|t| lower.contains(t.as_str()))
        })
        .map(|row| {
            let score = target.map(|t| compute_similarity_weighted(t, &row.envelope, cosine_weight));
            LibraryHit { row, score }
        })
        .collect();

    if target.is_some() {
        hits.sort_by(|a, b| {
            b.score
                .unwrap_or(0.0)
                .total_cmp(&a.score.unwrap_or(0.0))
                .then_with(|| a.row.path.cmp(&b.row.path))
        });
    } else {
        hits.sort_by(|a, b| a.row.path.to_lowercase().cmp(&b.row.path.to_lowercase()));
    }
    hits
}

pub fn query_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::envelope::normalize_envelope;
    use hound::{SampleFormat, WavSpec, WavWriter};
    use std::path::PathBuf;

    const SR: u32 = 22_050;

    /// Same shapes as the UI presets (`generatePresetCurve` in main.ts).
    fn preset(name: &str) -> Vec<f32> {
        let n = 64;
        let curve: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / (n - 1) as f32;
                match name {
                    "whoosh" => (-((t - 0.5) / 0.21).powi(2)).exp(),
                    "impact" => {
                        if i < 2 {
                            if i == 0 { 0.92 } else { 1.0 }
                        } else {
                            (-((t - 0.03) * 6.5)).exp()
                        }
                    }
                    "riser" => {
                        if t > 0.94 {
                            ((1.0 - t) / 0.06).max(0.15)
                        } else {
                            (t / 0.94).powf(2.3)
                        }
                    }
                    _ => unreachable!(),
                }
            })
            .collect();
        normalize_envelope(&curve)
    }

    /// Writes 1.5 s of a deterministic noise-like tone shaped by the preset curve.
    fn write_wav(path: &Path, shape: &[f32], seed: u32) {
        write_wav_secs(path, shape, seed, 1.5);
    }

    fn write_wav_secs(path: &Path, shape: &[f32], seed: u32, secs: f32) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: SR,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(path, spec).unwrap();
        let len = (SR as f32 * secs) as usize;
        let mut state = seed.max(1);
        for i in 0..len {
            let pos = i as f32 / (len - 1) as f32 * (shape.len() - 1) as f32;
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(shape.len() - 1);
            let amp = shape[lo] + (shape[hi] - shape[lo]) * (pos - lo as f32);
            // xorshift noise + low tone: keeps RMS roughly proportional to `amp`.
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let tone = (i as f32 * 220.0 * std::f32::consts::TAU / SR as f32).sin();
            let s = amp * (0.6 * tone + 0.4 * noise) * 0.8;
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sfx_de_babel_{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fixture_library(name: &str) -> (PathBuf, Database, String) {
        let dir = temp_dir(name);
        let audio = dir.join("audio");
        std::fs::create_dir_all(audio.join("nested")).unwrap();
        write_wav(&audio.join("whoosh_a.wav"), &preset("whoosh"), 11);
        write_wav(&audio.join("nested").join("impact_a.wav"), &preset("impact"), 22);
        write_wav(&audio.join("RISER_a.WAV"), &preset("riser"), 33);
        std::fs::write(audio.join("notes.txt"), "not audio").unwrap();
        let db = Database::open(&dir.join("db")).unwrap();
        let root = audio.to_str().unwrap().to_string();
        (dir, db, root)
    }

    #[test]
    fn ranking_finds_matching_synthetic_sounds_first() {
        let (dir, db, root) = fixture_library("lib_rank");
        let cancel = AtomicBool::new(false);
        let summary = index_library(&db, &root, &cancel, |_| {}).unwrap();
        assert_eq!(summary.total, 3, "nested + uppercase ext must be found, txt ignored");
        assert_eq!(summary.indexed, 3);
        assert_eq!(summary.failed, 0);

        for (name, expect) in [
            ("whoosh", "whoosh_a"),
            ("impact", "impact_a"),
            ("riser", "riser_a"),
        ] {
            let rows = db.library_rows(&root_prefix(&root)).unwrap();
            let hits = rank_library(rows, &[], Some(&preset(name)), 0.65);
            assert_eq!(hits.len(), 3);
            let top = file_title(&hits[0].row.path).to_lowercase();
            assert_eq!(top, expect, "target {name}: got {:?}", hits.iter().map(|h| (file_title(&h.row.path), h.score)).collect::<Vec<_>>());
            assert!(hits[0].score.unwrap() > hits[1].score.unwrap());
            assert!(hits[0].score.unwrap() > 80.0, "score {:?}", hits[0].score);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reindex_only_recomputes_changed_files_and_drops_removed() {
        let (dir, db, root) = fixture_library("lib_incremental");
        let cancel = AtomicBool::new(false);
        let first = index_library(&db, &root, &cancel, |_| {}).unwrap();
        assert_eq!((first.indexed, first.unchanged), (3, 0));

        let second = index_library(&db, &root, &cancel, |_| {}).unwrap();
        assert_eq!((second.indexed, second.unchanged, second.removed), (0, 3, 0));

        // Rewrite one file with a different length (so its size changes even on filesystems
        // with coarse mtime) and delete another.
        let audio = Path::new(&root);
        write_wav_secs(&audio.join("whoosh_a.wav"), &preset("whoosh"), 99, 2.5);
        std::fs::remove_file(audio.join("RISER_a.WAV")).unwrap();

        let third = index_library(&db, &root, &cancel, |_| {}).unwrap();
        assert_eq!(
            (third.total, third.indexed, third.unchanged, third.removed),
            (2, 1, 1, 1)
        );
        assert_eq!(db.library_count(&root_prefix(&root)).unwrap(), 2);
        let row = db
            .library_get(audio.join("whoosh_a.wav").to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!((row.duration - 2.5).abs() < 0.05, "duration {}", row.duration);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancel_flag_stops_indexing_without_rows() {
        let (dir, db, root) = fixture_library("lib_cancel");
        let cancel = AtomicBool::new(true);
        let summary = index_library(&db, &root, &cancel, |_| {}).unwrap();
        assert!(summary.cancelled);
        assert_eq!(summary.indexed, 0);
        assert_eq!(db.library_count(&root_prefix(&root)).unwrap(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn progress_reports_scanning_indexing_done() {
        let (dir, db, root) = fixture_library("lib_progress");
        let cancel = AtomicBool::new(false);
        let mut phases = Vec::new();
        index_library(&db, &root, &cancel, |p| phases.push((p.phase, p.processed, p.total))).unwrap();
        assert_eq!(phases.first().map(|p| p.0), Some("scanning"));
        assert_eq!(phases.last().map(|p| p.0), Some("done"));
        assert_eq!(phases.last().map(|p| (p.1, p.2)), Some((3, 3)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn root_prefix_does_not_match_sibling_folders() {
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(root_prefix("/music/"), format!("/music{sep}"));
        assert!(!format!("/music2{sep}a.wav").starts_with(&root_prefix("/music")));
    }

    #[test]
    fn text_tokens_filter_by_path_and_duration_relaxes_when_empty() {
        let mk = |p: &str, d: f32| LibraryRow {
            path: p.to_string(),
            mtime_ms: 0,
            size: 0,
            duration: d,
            envelope: vec![0.5; 64],
        };
        let rows = vec![mk("/a/Whoosh_big.wav", 1.0), mk("/a/hit.wav", 2.0)];
        let hits = rank_library(rows.clone(), &["whoosh".to_string()], None, 0.65);
        assert_eq!(hits.len(), 1);

        let (kept, relaxed) = filter_by_duration(rows.clone(), 0.5, 1.5);
        assert_eq!((kept.len(), relaxed), (1, false));
        let (kept, relaxed) = filter_by_duration(rows, 5.0, 6.0);
        assert_eq!((kept.len(), relaxed), (2, true));
    }
}
