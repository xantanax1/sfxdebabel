use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};

use crate::audio::decoder::DecodedAudio;
use crate::error::AppError;
use crate::models::PlaybackStatus;

enum PlayerCommand {
    Play {
        gen: u64,
        sound_id: String,
        samples: Vec<f32>,
        channels: u16,
        sample_rate: u32,
        start_secs: f32,
        total_duration_secs: f32,
        volume: Option<f32>,
        reply: mpsc::Sender<Result<bool, AppError>>,
    },
    Stop,
    SetVolume(f32),
}

struct PlaybackTracker {
    sound_id: Option<String>,
    started_at: Option<Instant>,
    offset_secs: f32,
    duration_secs: f32,
    is_playing: bool,
}

impl Default for PlaybackTracker {
    fn default() -> Self {
        Self {
            sound_id: None,
            started_at: None,
            offset_secs: 0.0,
            duration_secs: 0.0,
            is_playing: false,
        }
    }
}

/// Thread-safe audio preview controller. `rodio::OutputStream` is `!Send` on Windows/macOS,
/// so it lives inside a dedicated OS thread controlled over a channel.
pub struct AudioPlayer {
    tx: Mutex<mpsc::Sender<PlayerCommand>>,
    tracker: Arc<Mutex<PlaybackTracker>>,
    generation: Arc<AtomicU64>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<PlayerCommand>();
        let tracker = Arc::new(Mutex::new(PlaybackTracker::default()));
        let tracker_thread = Arc::clone(&tracker);
        let generation = Arc::new(AtomicU64::new(0));
        let generation_thread = Arc::clone(&generation);

        std::thread::Builder::new()
            .name("sfx-audio-player".to_string())
            .spawn(move || {
                let mut stream_pair = OutputStream::try_default().ok();
                let mut current_sink: Option<Sink> = None;
                let mut volume: f32 = 0.85;

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        PlayerCommand::Play {
                            gen,
                            sound_id,
                            samples,
                            channels,
                            sample_rate,
                            start_secs,
                            total_duration_secs,
                            volume: req_vol,
                            reply,
                        } => {
                            if let Some(v) = req_vol {
                                volume = v.clamp(0.0, 1.0);
                            }

                            if gen != generation_thread.load(Ordering::SeqCst) {
                                if let Some(sink) = current_sink.take() {
                                    sink.stop();
                                }
                                let _ = reply.send(Ok(false));
                                continue;
                            }

                            if stream_pair.is_none() {
                                stream_pair = OutputStream::try_default().ok();
                            }

                            let result = match &stream_pair {
                                None => Err(AppError::AudioPlayback(
                                    "Аудиоустройство вывода недоступно".to_string(),
                                )),
                                Some((_, handle)) => {
                                    if let Some(sink) = current_sink.take() {
                                        sink.stop();
                                    }
                                    if gen != generation_thread.load(Ordering::SeqCst) {
                                        Ok(false)
                                    } else {
                                        match Sink::try_new(handle) {
                                            Ok(sink) => {
                                                sink.set_volume(volume);
                                                let ch = channels.max(1) as usize;
                                                let total_frames = samples.len() / ch;
                                                let start_frame = ((start_secs.max(0.0)
                                                    * sample_rate as f32)
                                                    as usize)
                                                    .min(total_frames);
                                                let start_idx = start_frame * ch;
                                                let slice = samples[start_idx..].to_vec();
                                                let source = SamplesBuffer::new(
                                                    channels.max(1),
                                                    sample_rate.max(1),
                                                    slice,
                                                );
                                                sink.append(source);
                                                sink.play();
                                                current_sink = Some(sink);

                                                if let Ok(mut guard) = tracker_thread.lock() {
                                                    guard.sound_id = Some(sound_id);
                                                    guard.started_at = Some(Instant::now());
                                                    guard.offset_secs = start_secs.max(0.0);
                                                    guard.duration_secs =
                                                        total_duration_secs.max(0.01);
                                                    guard.is_playing = true;
                                                }
                                                Ok(true)
                                            }
                                            Err(e) => Err(AppError::AudioPlayback(format!(
                                                "Не удалось создать Sink: {e}"
                                            ))),
                                        }
                                    }
                                }
                            };

                            let _ = reply.send(result);
                        }
                        PlayerCommand::Stop => {
                            if let Some(sink) = current_sink.take() {
                                sink.stop();
                            }
                            if let Ok(mut guard) = tracker_thread.lock() {
                                guard.is_playing = false;
                                guard.started_at = None;
                            }
                        }
                        PlayerCommand::SetVolume(v) => {
                            volume = v.clamp(0.0, 1.0);
                            if let Some(sink) = &current_sink {
                                sink.set_volume(volume);
                            }
                        }
                    }
                }
            })
            .expect("failed to spawn audio player thread");

        Self {
            tx: Mutex::new(tx),
            tracker,
            generation,
        }
    }

    /// Starts a new playback request, immediately stopping any active sound and invalidating
    /// earlier async playback tasks that have not started playing yet.
    pub fn begin_request(&self) -> u64 {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut guard) = self.tracker.lock() {
            guard.is_playing = false;
            guard.started_at = None;
        }
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(PlayerCommand::Stop);
        }
        gen
    }

    pub fn is_current(&self, gen: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == gen
    }

    pub fn play_if_current(
        &self,
        gen: u64,
        sound_id: String,
        audio: &DecodedAudio,
        start_secs: Option<f32>,
        volume: Option<f32>,
    ) -> Result<Option<PlaybackStatus>, AppError> {
        if !self.is_current(gen) {
            return Ok(None);
        }
        let clamped_start = start_secs
            .unwrap_or(0.0)
            .clamp(0.0, (audio.duration_secs - 0.02).max(0.0));
        let (reply_tx, reply_rx) = mpsc::channel();

        let cmd = PlayerCommand::Play {
            gen,
            sound_id,
            samples: audio.interleaved_samples.clone(),
            channels: audio.channels,
            sample_rate: audio.sample_rate,
            start_secs: clamped_start,
            total_duration_secs: audio.duration_secs,
            volume,
            reply: reply_tx,
        };

        self.tx
            .lock()
            .map_err(|_| AppError::AudioPlayback("Ошибка блокировки плеера".to_string()))?
            .send(cmd)
            .map_err(|_| AppError::AudioPlayback("Аудиопоток остановлен".to_string()))?;

        let played = reply_rx
            .recv()
            .map_err(|_| AppError::AudioPlayback("Нет ответа от аудиопотока".to_string()))??;

        if played {
            Ok(Some(self.get_status()))
        } else {
            Ok(None)
        }
    }

    #[allow(dead_code)]
    pub fn play(
        &self,
        sound_id: String,
        audio: &DecodedAudio,
        start_secs: Option<f32>,
        volume: Option<f32>,
    ) -> Result<PlaybackStatus, AppError> {
        let gen = self.begin_request();
        let status = self.play_if_current(gen, sound_id, audio, start_secs, volume)?;
        Ok(status.unwrap_or_else(|| self.get_status()))
    }

    pub fn stop(&self) -> Result<(), AppError> {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut guard) = self.tracker.lock() {
            guard.is_playing = false;
            guard.started_at = None;
        }
        self.tx
            .lock()
            .map_err(|_| AppError::AudioPlayback("Ошибка блокировки плеера".to_string()))?
            .send(PlayerCommand::Stop)
            .map_err(|_| AppError::AudioPlayback("Аудиопоток остановлен".to_string()))?;
        Ok(())
    }

    pub fn set_volume(&self, volume: f32) -> Result<(), AppError> {
        self.tx
            .lock()
            .map_err(|_| AppError::AudioPlayback("Ошибка блокировки плеера".to_string()))?
            .send(PlayerCommand::SetVolume(volume))
            .map_err(|_| AppError::AudioPlayback("Аудиопоток остановлен".to_string()))?;
        Ok(())
    }

    pub fn get_status(&self) -> PlaybackStatus {
        let Ok(mut guard) = self.tracker.lock() else {
            return PlaybackStatus {
                is_playing: false,
                sound_id: None,
                position_secs: 0.0,
                duration_secs: 0.0,
            };
        };

        if guard.is_playing {
            let elapsed = guard
                .started_at
                .map(|t| t.elapsed().as_secs_f32())
                .unwrap_or(0.0);
            let pos = guard.offset_secs + elapsed;
            if pos >= guard.duration_secs {
                guard.is_playing = false;
                guard.started_at = None;
                return PlaybackStatus {
                    is_playing: false,
                    sound_id: guard.sound_id.clone(),
                    position_secs: guard.duration_secs,
                    duration_secs: guard.duration_secs,
                };
            }
            return PlaybackStatus {
                is_playing: true,
                sound_id: guard.sound_id.clone(),
                position_secs: pos,
                duration_secs: guard.duration_secs,
            };
        }

        PlaybackStatus {
            is_playing: false,
            sound_id: guard.sound_id.clone(),
            position_secs: guard.offset_secs,
            duration_secs: guard.duration_secs,
        }
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_audio() -> DecodedAudio {
        DecodedAudio {
            mono_samples: vec![0.0; 1024],
            interleaved_samples: vec![0.0; 1024],
            sample_rate: 44_100,
            channels: 1,
            duration_secs: 1024.0 / 44_100.0,
        }
    }

    #[test]
    fn stop_and_new_request_invalidate_stale_generation() {
        let player = AudioPlayer::new();
        let g1 = player.begin_request();
        assert!(player.is_current(g1));

        let g2 = player.begin_request();
        assert!(!player.is_current(g1));
        assert!(player.is_current(g2));

        let stale = player
            .play_if_current(g1, "old".to_string(), &dummy_audio(), None, None)
            .unwrap();
        assert!(stale.is_none());

        player.stop().unwrap();
        assert!(!player.is_current(g2));
        let stopped = player
            .play_if_current(g2, "stopped".to_string(), &dummy_audio(), None, None)
            .unwrap();
        assert!(stopped.is_none());
        assert!(!player.get_status().is_playing);
    }
}
