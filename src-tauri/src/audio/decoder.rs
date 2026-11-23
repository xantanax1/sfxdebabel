use std::io::Cursor;

use bytes::Bytes;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub mono_samples: Vec<f32>,
    pub interleaved_samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_secs: f32,
}

/// Decodes an in-memory audio container (MP3, OGG/Vorbis, WAV, FLAC) into f32 PCM
/// without writing temporary files to disk.
pub fn decode_audio_bytes(data: Bytes, ext_hint: Option<&str>) -> Result<DecodedAudio, AppError> {
    if data.is_empty() {
        return Err(AppError::AudioDecode("Пустой аудиобуфер".to_string()));
    }

    let cursor = Cursor::new(data);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = ext_hint.filter(|e| !e.trim().is_empty()) {
        hint.with_extension(ext.trim().trim_start_matches('.'));
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AppError::AudioDecode(format!("Не удалось распознать формат: {e}")))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AppError::AudioDecode("Аудиодорожка не найдена".to_string()))?;

    let track_id = track.id;
    let initial_sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let initial_channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AppError::AudioDecode(format!("Неподдерживаемый кодек: {e}")))?;

    let mut sample_rate = initial_sample_rate;
    let mut channels = initial_channels;
    let mut mono_samples: Vec<f32> = Vec::new();
    let mut interleaved_samples: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(err) => {
                if !mono_samples.is_empty() {
                    break;
                }
                return Err(AppError::AudioDecode(format!("Ошибка чтения пакета: {err}")));
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => {
                if !mono_samples.is_empty() {
                    break;
                }
                return Err(AppError::AudioDecode(format!("Ошибка декодирования: {err}")));
            }
        };

        let spec = *decoded.spec();
        sample_rate = spec.rate.max(1);
        let frame_channels = spec.channels.count().max(1);
        channels = frame_channels as u16;

        let buf = sample_buf.get_or_insert_with(|| {
            SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)
        });
        if buf.capacity() < decoded.capacity() {
            *buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        }

        buf.copy_interleaved_ref(decoded);
        let slice = buf.samples();
        interleaved_samples.extend_from_slice(slice);

        // Downmix interleaved channels to mono by averaging samples per frame.
        if frame_channels == 1 {
            mono_samples.extend_from_slice(slice);
        } else {
            let inv = 1.0 / frame_channels as f32;
            for frame in slice.chunks_exact(frame_channels) {
                let sum: f32 = frame.iter().copied().sum();
                mono_samples.push(sum * inv);
            }
        }
    }

    if mono_samples.is_empty() {
        return Err(AppError::AudioDecode(
            "Аудиофайл не содержит сэмплов".to_string(),
        ));
    }

    let duration_secs = mono_samples.len() as f32 / sample_rate as f32;

    Ok(DecodedAudio {
        mono_samples,
        interleaved_samples,
        sample_rate,
        channels,
        duration_secs,
    })
}
