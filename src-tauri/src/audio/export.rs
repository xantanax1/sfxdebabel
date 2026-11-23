use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};

use crate::audio::decoder::DecodedAudio;
use crate::error::AppError;

pub fn export_temp_dir() -> PathBuf {
    std::env::temp_dir().join("sfx_de_babel")
}

/// Attribution data embedded into exported WAV files (RIFF `LIST`/`INFO` chunk).
#[derive(Debug, Clone, Default)]
pub struct ExportMeta {
    pub title: String,
    pub author: String,
    pub license_name: String,
    pub license_url: String,
    pub source_url: String,
}

fn info_chunk(id: &[u8; 4], value: &str, out: &mut Vec<u8>) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let mut data: Vec<u8> = value.replace('\0', "").into_bytes();
    data.push(0); // INFO strings are NUL-terminated.
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// Builds the bytes of a `LIST`/`INFO` chunk, or `None` when there is nothing to write.
fn build_info_list(meta: &ExportMeta) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(b"INFO");
    info_chunk(b"INAM", &meta.title, &mut body);
    info_chunk(b"IART", &meta.author, &mut body);

    let mut comment = String::new();
    if !meta.license_name.trim().is_empty() {
        comment.push_str(&format!("License: {}", meta.license_name.trim()));
        if !meta.license_url.trim().is_empty() {
            comment.push_str(&format!(" ({})", meta.license_url.trim()));
        }
    }
    if !meta.source_url.trim().is_empty() {
        if !comment.is_empty() {
            comment.push_str(". ");
        }
        comment.push_str(&format!("Source: {}", meta.source_url.trim()));
    }
    info_chunk(b"ICMT", &comment, &mut body);
    info_chunk(b"ICOP", &meta.license_name, &mut body);
    info_chunk(b"ISRC", &meta.source_url, &mut body);

    if body.len() <= 4 {
        return None;
    }
    let mut list = Vec::with_capacity(body.len() + 8);
    list.extend_from_slice(b"LIST");
    list.extend_from_slice(&(body.len() as u32).to_le_bytes());
    list.extend_from_slice(&body);
    Some(list)
}

fn read_u32_le(bytes: &[u8], pos: usize) -> usize {
    u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize
}

/// Appends a `LIST`/`INFO` chunk after the `data` chunk of a finished WAV file and
/// fixes the RIFF size. hound cannot write INFO chunks, so this is done by hand.
fn append_info_chunk(path: &Path, meta: &ExportMeta) -> Result<(), AppError> {
    let Some(list) = build_info_list(meta) else {
        return Ok(());
    };
    let mut bytes = std::fs::read(path)?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AppError::AudioExport("Некорректный WAV-заголовок".to_string()));
    }

    // Walk chunks to find where the data chunk ends.
    let mut pos = 12usize;
    let mut data_end = None;
    while pos + 8 <= bytes.len() {
        let size = read_u32_le(&bytes, pos + 4);
        let body_end = pos + 8 + size;
        if &bytes[pos..pos + 4] == b"data" {
            data_end = Some(body_end);
            break;
        }
        pos = body_end + (size & 1);
    }
    let data_end = data_end
        .filter(|&e| e <= bytes.len())
        .ok_or_else(|| AppError::AudioExport("WAV без data-чанка".to_string()))?;

    bytes.truncate(data_end);
    if data_end % 2 == 1 {
        bytes.push(0); // RIFF chunks are word aligned.
    }
    bytes.extend_from_slice(&list);
    let riff_size = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Reads INFO tags back from a WAV file (used by tests and diagnostics).
#[allow(dead_code)]
pub fn read_info_tags(bytes: &[u8]) -> Vec<(String, String)> {
    let mut tags = Vec::new();
    if bytes.len() < 12 {
        return tags;
    }
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let size = read_u32_le(bytes, pos + 4);
        let body = pos + 8;
        let body_end = (body + size).min(bytes.len());
        if &bytes[pos..pos + 4] == b"LIST" && body_end >= body + 4 && &bytes[body..body + 4] == b"INFO"
        {
            let mut p = body + 4;
            while p + 8 <= body_end {
                let id = String::from_utf8_lossy(&bytes[p..p + 4]).into_owned();
                let sz = read_u32_le(bytes, p + 4);
                let v_end = (p + 8 + sz).min(body_end);
                let text = String::from_utf8_lossy(&bytes[p + 8..v_end])
                    .trim_end_matches('\0')
                    .to_string();
                tags.push((id, text));
                p = p + 8 + sz + (sz & 1);
            }
        }
        pos = body + size + (size & 1);
    }
    tags
}

fn sanitize_slug(input: &str, max_len: usize, fallback: &str) -> String {
    let mut out = String::with_capacity(input.len().min(max_len));
    let mut prev_underscore = false;

    for ch in input.chars() {
        if out.len() >= max_len {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if (ch == '-' || ch == '_' || ch.is_whitespace()) && !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }

    while out.ends_with('_') {
        out.pop();
    }

    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

/// Writes decoded audio samples to a 16-bit PCM WAV inside `temp_dir/sfx_de_babel/`
/// for direct OS drag-and-drop into NLEs and DAWs.
pub fn export_to_wav(
    sound_id: &str,
    title: &str,
    audio: &DecodedAudio,
    meta: Option<&ExportMeta>,
) -> Result<PathBuf, AppError> {
    let dir = export_temp_dir();
    export_to_wav_in_dir(&dir, sound_id, title, audio, meta)
}

pub fn export_to_wav_in_dir(
    dir: &Path,
    sound_id: &str,
    title: &str,
    audio: &DecodedAudio,
    meta: Option<&ExportMeta>,
) -> Result<PathBuf, AppError> {
    std::fs::create_dir_all(dir)?;

    let safe_title = sanitize_slug(title, 44, "sound");
    let safe_id = sanitize_slug(sound_id, 24, "item");
    let filename = format!("{safe_title}_{safe_id}.wav");
    let out_path = dir.join(filename);

    if out_path.exists()
        && std::fs::metadata(&out_path)
            .map(|m| m.len() > 44)
            .unwrap_or(false)
    {
        return Ok(out_path);
    }

    let spec = WavSpec {
        channels: audio.channels.max(1),
        sample_rate: audio.sample_rate.max(8_000),
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let tmp_path = dir.join(format!(".tmp_{safe_id}.wav"));
    {
        let mut writer = WavWriter::create(&tmp_path, spec)
            .map_err(|e| AppError::AudioExport(format!("Не удалось создать WAV: {e}")))?;

        for &sample in &audio.interleaved_samples {
            let clamped = sample.clamp(-1.0, 1.0);
            let quantized = (clamped * i16::MAX as f32).round() as i16;
            writer
                .write_sample(quantized)
                .map_err(|e| AppError::AudioExport(format!("Ошибка записи сэмплов WAV: {e}")))?;
        }

        writer
            .finalize()
            .map_err(|e| AppError::AudioExport(format!("Ошибка финализации WAV: {e}")))?;
    }

    if let Some(meta) = meta {
        append_info_chunk(&tmp_path, meta)?;
    }
    std::fs::rename(&tmp_path, &out_path)?;
    Ok(out_path)
}

/// Writes FX-processed audio samples to a 24-bit PCM WAV inside `temp_dir/sfx_de_babel/`
/// with a descriptive FX suffix in the filename for NLE/DAW drag-and-drop.
pub fn export_to_wav_24bit(
    sound_id: &str,
    title: &str,
    fx_suffix: &str,
    audio: &DecodedAudio,
    meta: Option<&ExportMeta>,
) -> Result<PathBuf, AppError> {
    let dir = export_temp_dir();
    export_to_wav_24bit_in_dir(&dir, sound_id, title, fx_suffix, audio, meta)
}

pub fn export_to_wav_24bit_in_dir(
    dir: &Path,
    sound_id: &str,
    title: &str,
    fx_suffix: &str,
    audio: &DecodedAudio,
    meta: Option<&ExportMeta>,
) -> Result<PathBuf, AppError> {
    std::fs::create_dir_all(dir)?;

    let safe_title = sanitize_slug(title, 44, "sound");
    let safe_id = sanitize_slug(sound_id, 24, "item");
    let safe_suffix = sanitize_slug(fx_suffix, 48, "");

    let filename = if safe_suffix.is_empty() {
        format!("{safe_title}_{safe_id}.wav")
    } else {
        format!("{safe_title}_{safe_suffix}.wav")
    };
    let out_path = dir.join(filename);

    let spec = WavSpec {
        channels: audio.channels.max(1),
        sample_rate: audio.sample_rate.max(8_000),
        bits_per_sample: 24,
        sample_format: SampleFormat::Int,
    };

    let tmp_path = dir.join(format!(".tmp24_{safe_id}.wav"));
    {
        let mut writer = WavWriter::create(&tmp_path, spec)
            .map_err(|e| AppError::AudioExport(format!("Не удалось создать 24-bit WAV: {e}")))?;

        // 24-bit signed integer range is [-8_388_608, 8_388_607].
        const MAX_24BIT: f32 = 8_388_607.0;
        for &sample in &audio.interleaved_samples {
            let clamped = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let quantized = (clamped * MAX_24BIT).round() as i32;
            writer
                .write_sample(quantized)
                .map_err(|e| AppError::AudioExport(format!("Ошибка записи 24-bit WAV: {e}")))?;
        }

        writer
            .finalize()
            .map_err(|e| AppError::AudioExport(format!("Ошибка финализации 24-bit WAV: {e}")))?;
    }

    if let Some(meta) = meta {
        append_info_chunk(&tmp_path, meta)?;
    }

    if out_path.exists() {
        let _ = std::fs::remove_file(&out_path);
    }
    std::fs::rename(&tmp_path, &out_path)?;
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decoder::decode_audio_bytes;
    use crate::audio::envelope::compute_rms_envelope;
    use crate::audio::fx::{AudioBuffer, FxChain, FxParams};
    use bytes::Bytes;

    #[test]
    fn wav_export_and_symphonia_roundtrip() {
        let sample_rate = 44_100_u32;
        let len = 22_050_usize; // 0.5s
        let mut samples = Vec::with_capacity(len);
        for i in 0..len {
            let t = i as f32 / sample_rate as f32;
            samples.push((t * 440.0 * std::f32::consts::TAU).sin() * 0.6);
        }

        let audio = DecodedAudio {
            mono_samples: samples.clone(),
            interleaved_samples: samples,
            sample_rate,
            channels: 1,
            duration_secs: 0.5,
        };

        let temp_dir = std::env::temp_dir().join("sfx_de_babel_unit_test");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let wav_path =
            export_to_wav_in_dir(&temp_dir, "test_440", "Air Whoosh! (Test)", &audio, None).unwrap();
        assert!(wav_path.exists());

        let raw_bytes = Bytes::from(std::fs::read(&wav_path).unwrap());
        let decoded = decode_audio_bytes(raw_bytes, Some("wav")).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert!((decoded.duration_secs - 0.5).abs() < 0.02);

        let env = compute_rms_envelope(&decoded.mono_samples);
        assert_eq!(env.len(), 64);
        assert!(env[32] > 0.85);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn wav_24bit_fx_export_roundtrip() {
        let sample_rate = 44_100_u32;
        let len = 22_050_usize;
        let mut samples = Vec::with_capacity(len);
        for i in 0..len {
            let t = i as f32 / sample_rate as f32;
            samples.push((t * 220.0 * std::f32::consts::TAU).sin() * 0.5);
        }

        let fx = FxParams {
            speed_pitch: 0.8,
            reverse: true,
            ..Default::default()
        };
        let suffix = fx.filename_suffix();
        assert!(suffix.starts_with("slowed_rev_"));

        let processed = FxChain::process(
            &AudioBuffer {
                samples,
                sample_rate,
                channels: 1,
            },
            &fx,
        )
        .to_decoded();

        let temp_dir = std::env::temp_dir().join("sfx_de_babel_unit_test_24bit");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let wav_path =
            export_to_wav_24bit_in_dir(&temp_dir, "mk_99", "Air Whoosh", &suffix, &processed, None)
                .unwrap();
        assert!(wav_path.exists());
        let fname = wav_path.file_name().unwrap().to_string_lossy();
        assert!(fname.starts_with("air_whoosh_slowed_rev_"));
        assert!(fname.ends_with(".wav"));

        let raw_bytes = Bytes::from(std::fs::read(&wav_path).unwrap());
        let decoded = decode_audio_bytes(raw_bytes, Some("wav")).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert!((decoded.duration_secs - 0.625).abs() < 0.03);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn wav_info_chunk_roundtrip_24bit_and_16bit() {
        let sample_rate = 44_100_u32;
        // 1001 mono samples -> odd 16-bit data length is even, 24-bit is odd (3003): exercises padding.
        let samples: Vec<f32> = (0..1001)
            .map(|i| (i as f32 / sample_rate as f32 * 330.0 * std::f32::consts::TAU).sin() * 0.4)
            .collect();
        let audio = DecodedAudio {
            mono_samples: samples.clone(),
            interleaved_samples: samples,
            sample_rate,
            channels: 1,
            duration_secs: 1001.0 / sample_rate as f32,
        };
        let meta = ExportMeta {
            title: "Air Whoosh".to_string(),
            author: "FreqMan".to_string(),
            license_name: "CC BY 4.0".to_string(),
            license_url: "https://creativecommons.org/licenses/by/4.0/".to_string(),
            source_url: "https://freesound.org/s/25071/".to_string(),
        };

        let dir = std::env::temp_dir().join("sfx_de_babel_unit_test_info");
        let _ = std::fs::remove_dir_all(&dir);

        let p24 =
            export_to_wav_24bit_in_dir(&dir, "fs_25071", "Air Whoosh", "", &audio, Some(&meta))
                .unwrap();
        let p16 =
            export_to_wav_in_dir(&dir, "fs_25071", "Air Whoosh", &audio, Some(&meta)).unwrap();

        for path in [&p24, &p16] {
            let bytes = std::fs::read(path).unwrap();
            let riff = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
            assert_eq!(riff + 8, bytes.len(), "RIFF size must match file length");

            let tags: std::collections::HashMap<_, _> =
                read_info_tags(&bytes).into_iter().collect();
            assert_eq!(tags.get("IART").map(String::as_str), Some("FreqMan"));
            assert_eq!(tags.get("INAM").map(String::as_str), Some("Air Whoosh"));
            assert_eq!(
                tags.get("ISRC").map(String::as_str),
                Some("https://freesound.org/s/25071/")
            );
            let icmt = tags.get("ICMT").expect("ICMT present");
            assert!(icmt.contains("CC BY 4.0"));
            assert!(icmt.contains("https://freesound.org/s/25071/"));

            // Audio must stay readable by both hound and symphonia.
            let reader = hound::WavReader::open(path).unwrap();
            assert_eq!(reader.len() as usize, 1001);
            let decoded = decode_audio_bytes(Bytes::from(bytes), Some("wav")).unwrap();
            assert_eq!(decoded.interleaved_samples.len(), 1001);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_meta_writes_no_info_chunk() {
        assert!(build_info_list(&ExportMeta::default()).is_none());
    }
}
