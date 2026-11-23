use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};

use crate::audio::decoder::DecodedAudio;
use crate::error::AppError;

pub fn export_temp_dir() -> PathBuf {
    std::env::temp_dir().join("sfx_de_babel")
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
pub fn export_to_wav(sound_id: &str, title: &str, audio: &DecodedAudio) -> Result<PathBuf, AppError> {
    let dir = export_temp_dir();
    export_to_wav_in_dir(&dir, sound_id, title, audio)
}

pub fn export_to_wav_in_dir(
    dir: &Path,
    sound_id: &str,
    title: &str,
    audio: &DecodedAudio,
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
) -> Result<PathBuf, AppError> {
    let dir = export_temp_dir();
    export_to_wav_24bit_in_dir(&dir, sound_id, title, fx_suffix, audio)
}

pub fn export_to_wav_24bit_in_dir(
    dir: &Path,
    sound_id: &str,
    title: &str,
    fx_suffix: &str,
    audio: &DecodedAudio,
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
            export_to_wav_in_dir(&temp_dir, "test_440", "Air Whoosh! (Test)", &audio).unwrap();
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
            export_to_wav_24bit_in_dir(&temp_dir, "mk_99", "Air Whoosh", &suffix, &processed)
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
}
