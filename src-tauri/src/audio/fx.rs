use std::f32::consts::{FRAC_1_SQRT_2, PI};

use serde::{Deserialize, Serialize};

use crate::audio::decoder::DecodedAudio;

/// Interleaved f32 audio buffer processed non-destructively by [`FxChain`].
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioBuffer {
    pub fn from_decoded(decoded: &DecodedAudio) -> Self {
        Self {
            samples: decoded.interleaved_samples.clone(),
            sample_rate: decoded.sample_rate.max(8_000),
            channels: decoded.channels.max(1),
        }
    }

    pub fn to_mono(&self) -> Vec<f32> {
        let ch = self.channels.max(1) as usize;
        if ch == 1 {
            return self.samples.clone();
        }
        let inv = 1.0 / ch as f32;
        self.samples
            .chunks_exact(ch)
            .map(|frame| frame.iter().copied().sum::<f32>() * inv)
            .collect()
    }

    pub fn to_stereo(&self) -> Self {
        let ch = self.channels.max(1) as usize;
        if ch == 2 {
            return self.clone();
        }
        if ch == 1 {
            let mut out = Vec::with_capacity(self.samples.len() * 2);
            for &s in &self.samples {
                out.push(s);
                out.push(s);
            }
            return Self {
                samples: out,
                sample_rate: self.sample_rate,
                channels: 2,
            };
        }
        // Downmix >2 channels into stereo L/R pair
        let mut out = Vec::with_capacity((self.samples.len() / ch) * 2);
        for frame in self.samples.chunks_exact(ch) {
            out.push(frame[0]);
            out.push(frame[1]);
        }
        Self {
            samples: out,
            sample_rate: self.sample_rate,
            channels: 2,
        }
    }

    pub fn to_decoded(&self) -> DecodedAudio {
        let mono_samples = self.to_mono();
        let duration_secs = mono_samples.len() as f32 / self.sample_rate.max(1) as f32;
        DecodedAudio {
            mono_samples,
            interleaved_samples: self.samples.clone(),
            sample_rate: self.sample_rate,
            channels: self.channels.max(1),
            duration_secs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeedMode {
    #[default]
    PitchAndTempo,
    TempoOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FxParams {
    pub speed_pitch: f32,
    pub speed_mode: SpeedMode,
    pub reverse: bool,
    pub reverb_mix: f32,
    pub reverb_room_size: f32,
    pub reverb_damping: f32,
    pub lowpass_hz: f32,
    pub highpass_hz: f32,
    pub drive: f32,
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
    pub normalize: bool,
}

impl Default for FxParams {
    fn default() -> Self {
        Self {
            speed_pitch: 1.0,
            speed_mode: SpeedMode::PitchAndTempo,
            reverse: false,
            reverb_mix: 0.0,
            reverb_room_size: 0.65,
            reverb_damping: 0.45,
            lowpass_hz: 20_000.0,
            highpass_hz: 20.0,
            drive: 0.0,
            fade_in_ms: 0,
            fade_out_ms: 0,
            normalize: true,
        }
    }
}

impl FxParams {
    pub fn sanitized(&self) -> Self {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        Self {
            speed_pitch: finite(self.speed_pitch, 1.0).clamp(0.5, 2.0),
            speed_mode: self.speed_mode,
            reverse: self.reverse,
            reverb_mix: finite(self.reverb_mix, 0.0).clamp(0.0, 1.0),
            reverb_room_size: finite(self.reverb_room_size, 0.65).clamp(0.1, 1.0),
            reverb_damping: finite(self.reverb_damping, 0.45).clamp(0.0, 1.0),
            lowpass_hz: finite(self.lowpass_hz, 20_000.0).clamp(200.0, 20_000.0),
            highpass_hz: finite(self.highpass_hz, 20.0).clamp(20.0, 5_000.0),
            drive: finite(self.drive, 0.0).clamp(0.0, 1.0),
            fade_in_ms: self.fade_in_ms.min(5_000),
            fade_out_ms: self.fade_out_ms.min(5_000),
            normalize: self.normalize,
        }
    }

    pub fn is_fx_clean(&self) -> bool {
        let s = self.sanitized();
        (s.speed_pitch - 1.0).abs() < 0.005
            && s.speed_mode == SpeedMode::PitchAndTempo
            && !s.reverse
            && s.reverb_mix < 0.005
            && s.lowpass_hz >= 19_950.0
            && s.highpass_hz <= 21.0
            && s.drive < 0.005
            && s.fade_in_ms == 0
            && s.fade_out_ms == 0
    }

    /// Generates a descriptive, collision-safe filename suffix from active FX settings.
    pub fn filename_suffix(&self) -> String {
        let s = self.sanitized();
        if s.is_fx_clean() && s.normalize {
            return String::new();
        }

        let mut tags: Vec<&'static str> = Vec::new();
        if s.speed_pitch < 0.98 {
            tags.push("slowed");
            if s.speed_mode == SpeedMode::TempoOnly {
                tags.push("tempo");
            }
        } else if s.speed_pitch > 1.02 {
            tags.push("speedup");
            if s.speed_mode == SpeedMode::TempoOnly {
                tags.push("tempo");
            }
        }
        if s.reverse {
            tags.push("rev");
        }
        if s.reverb_mix >= 0.01 {
            tags.push("verb");
        }
        if s.lowpass_hz < 19_500.0 {
            tags.push("lpf");
        }
        if s.highpass_hz > 25.0 {
            tags.push("hpf");
        }
        if s.drive >= 0.01 {
            tags.push("drive");
        }
        if s.fade_in_ms > 0 || s.fade_out_ms > 0 {
            tags.push("fade");
        }
        if tags.is_empty() {
            if !s.normalize {
                tags.push("raw");
            } else {
                tags.push("tempo");
            }
        }

        // 32-bit FNV-1a hash of quantized parameter values so changing any knob yields a unique WAV path.
        let mut hash: u32 = 0x811c_9dc5;
        let parts: [u32; 12] = [
            (s.speed_pitch * 100.0).round() as u32,
            u32::from(s.speed_mode == SpeedMode::TempoOnly),
            u32::from(s.reverse),
            (s.reverb_mix * 100.0).round() as u32,
            (s.reverb_room_size * 100.0).round() as u32,
            (s.reverb_damping * 100.0).round() as u32,
            s.lowpass_hz.round() as u32,
            s.highpass_hz.round() as u32,
            (s.drive * 100.0).round() as u32,
            s.fade_in_ms,
            s.fade_out_ms,
            u32::from(s.normalize),
        ];
        for p in parts {
            for b in p.to_le_bytes() {
                hash ^= u32::from(b);
                hash = hash.wrapping_mul(0x0100_0193);
            }
        }

        format!("{}_{:06x}", tags.join("_"), hash & 0x00ff_ffff)
    }
}

/// Second-order biquad IIR filter (Audio EQ Cookbook, Direct Form II Transposed).
#[derive(Clone)]
struct BiquadChannel {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl BiquadChannel {
    fn lowpass(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let sr = sample_rate.max(8_000.0);
        let fc = cutoff_hz.clamp(20.0, sr * 0.45);
        let w0 = 2.0 * PI * fc / sr;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;

        Self {
            b0: ((1.0 - cos_w0) * 0.5) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn highpass(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let sr = sample_rate.max(8_000.0);
        let fc = cutoff_hz.clamp(10.0, sr * 0.45);
        let w0 = 2.0 * PI * fc / sr;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;

        Self {
            b0: ((1.0 + cos_w0) * 0.5) / a0,
            b1: -(1.0 + cos_w0) / a0,
            b2: ((1.0 + cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        if self.z1.abs() < 1e-20 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-20 {
            self.z2 = 0.0;
        }
        y
    }
}

struct CombFilter {
    buffer: Vec<f32>,
    idx: usize,
    filter_store: f32,
    feedback: f32,
    damp1: f32,
    damp2: f32,
}

impl CombFilter {
    fn new(len: usize, feedback: f32, damp: f32) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            idx: 0,
            filter_store: 0.0,
            feedback,
            damp1: damp,
            damp2: 1.0 - damp,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.idx];
        self.filter_store = output * self.damp2 + self.filter_store * self.damp1;
        if self.filter_store.abs() < 1e-20 {
            self.filter_store = 0.0;
        }
        self.buffer[self.idx] = input + self.filter_store * self.feedback;
        self.idx += 1;
        if self.idx >= self.buffer.len() {
            self.idx = 0;
        }
        output
    }
}

struct AllpassFilter {
    buffer: Vec<f32>,
    idx: usize,
    feedback: f32,
}

impl AllpassFilter {
    fn new(len: usize, feedback: f32) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            idx: 0,
            feedback,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buf_out = self.buffer[self.idx];
        let output = -input + buf_out;
        self.buffer[self.idx] = input + buf_out * self.feedback;
        self.idx += 1;
        if self.idx >= self.buffer.len() {
            self.idx = 0;
        }
        output
    }
}

/// Non-destructive pure-Rust audio FX processor.
pub struct FxChain;

impl FxChain {
    pub fn process(input: &AudioBuffer, raw_params: &FxParams) -> AudioBuffer {
        let params = raw_params.sanitized();
        let sr = input.sample_rate.max(8_000);
        let ch = input.channels.max(1) as usize;

        if input.samples.is_empty() {
            return input.clone();
        }

        // Sanitize non-finite samples and truncate to complete interleaved frames.
        let frame_count = input.samples.len() / ch;
        let mut samples: Vec<f32> = input.samples[..frame_count * ch]
            .iter()
            .map(|&s| if s.is_finite() { s } else { 0.0 })
            .collect();

        // 1. Reverse frame-by-frame while preserving channel layout [L, R].
        if params.reverse {
            let mut rev = Vec::with_capacity(samples.len());
            for frame in samples.chunks_exact(ch).rev() {
                rev.extend_from_slice(frame);
            }
            samples = rev;
        }

        // 2. Speed adjustment: Pitch+Tempo (Catmull-Rom) or Tempo-Only (WSOLA).
        if (params.speed_pitch - 1.0).abs() >= 0.005 {
            match params.speed_mode {
                SpeedMode::PitchAndTempo => {
                    // Pre-filter before downsampling when speed > 1.0 to avoid aliasing foldback.
                    if params.speed_pitch > 1.02 {
                        let nyquist_guard = (sr as f32 * 0.45) / params.speed_pitch;
                        apply_biquad_in_place(&mut samples, ch, || {
                            BiquadChannel::lowpass(sr as f32, nyquist_guard, FRAC_1_SQRT_2)
                        });
                    }
                    samples = resample_catmull_rom(&samples, ch, params.speed_pitch);
                }
                SpeedMode::TempoOnly => {
                    samples = time_stretch_wsola(&samples, ch, sr, params.speed_pitch);
                }
            }
        }

        // 3. Second-order biquad EQ filters (HPF -> LPF).
        if params.highpass_hz > 21.0 {
            apply_biquad_in_place(&mut samples, ch, || {
                BiquadChannel::highpass(sr as f32, params.highpass_hz, FRAC_1_SQRT_2)
            });
        }
        if params.lowpass_hz < 19_950.0 {
            apply_biquad_in_place(&mut samples, ch, || {
                BiquadChannel::lowpass(sr as f32, params.lowpass_hz, FRAC_1_SQRT_2)
            });
        }

        // 4. Soft tanh saturator with gain compensation.
        if params.drive >= 0.005 {
            let gain = 1.0 + params.drive * 8.0;
            let comp = 1.0 / gain.tanh();
            for s in &mut samples {
                *s = (*s * gain).tanh() * comp;
            }
        }

        let mut buf = AudioBuffer {
            samples,
            sample_rate: sr,
            channels: ch as u16,
        };

        // 5. Algorithmic stereo Freeverb (upmixes mono sources to true stereo).
        if params.reverb_mix >= 0.005 {
            buf = apply_stereo_freeverb(
                &buf,
                params.reverb_mix,
                params.reverb_room_size,
                params.reverb_damping,
            );
        }

        // 6. Raised-cosine Fade In / Fade Out to prevent clicks.
        let out_ch = buf.channels.max(1) as usize;
        apply_fades(&mut buf.samples, out_ch, sr, params.fade_in_ms, params.fade_out_ms);

        // 7. Peak normalization (-0.3 dBFS) and smooth safety limiter.
        apply_normalize_and_limiter(&mut buf.samples, params.normalize);

        buf
    }
}

fn apply_biquad_in_place<F>(samples: &mut [f32], channels: usize, make_filter: F)
where
    F: Fn() -> BiquadChannel,
{
    let mut filters: Vec<BiquadChannel> = (0..channels).map(|_| make_filter()).collect();
    for frame in samples.chunks_exact_mut(channels) {
        for (c, sample) in frame.iter_mut().enumerate() {
            *sample = filters[c].process(*sample);
        }
    }
}

fn resample_catmull_rom(samples: &[f32], channels: usize, speed_pitch: f32) -> Vec<f32> {
    let in_frames = samples.len() / channels;
    if in_frames < 2 {
        return samples.to_vec();
    }
    let out_frames = ((in_frames as f64 / speed_pitch as f64).round() as usize).max(1);
    let mut out = Vec::with_capacity(out_frames * channels);
    let max_idx = (in_frames - 1) as isize;

    for out_i in 0..out_frames {
        let src_pos = out_i as f64 * speed_pitch as f64;
        let i1 = (src_pos.floor() as isize).clamp(0, max_idx);
        let t = (src_pos - i1 as f64) as f32;

        let i0 = (i1 - 1).clamp(0, max_idx) as usize;
        let u1 = i1 as usize;
        let i2 = (i1 + 1).clamp(0, max_idx) as usize;
        let i3 = (i1 + 2).clamp(0, max_idx) as usize;

        for c in 0..channels {
            let y0 = samples[i0 * channels + c];
            let y1 = samples[u1 * channels + c];
            let y2 = samples[i2 * channels + c];
            let y3 = samples[i3 * channels + c];

            // Catmull-Rom spline coefficients ensure C1 continuity without linear high-frequency rolloff.
            let a0 = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
            let a1 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let a2 = -0.5 * y0 + 0.5 * y2;
            let a3 = y1;

            let val = ((a0 * t + a1) * t + a2) * t + a3;
            out.push(val);
        }
    }

    out
}

/// Pure-Rust WSOLA (Waveform Similarity Overlap-Add) time-stretcher without pitch change.
/// Uses joint multi-channel normalized cross-correlation so stereo channels stay phase-locked.
fn time_stretch_wsola(
    samples: &[f32],
    channels: usize,
    sample_rate: u32,
    speed: f32,
) -> Vec<f32> {
    let ch = channels.max(1);
    let in_frames = samples.len() / ch;
    if in_frames < 64 {
        return resample_catmull_rom(samples, ch, speed);
    }

    let out_frames = ((in_frames as f64 / speed as f64).round() as usize).max(1);
    let desired_win = (sample_rate as f32 * 0.040).round() as usize;
    let mut win = desired_win.min((in_frames / 2) & !1).max(32);
    win &= !1; // Keep window even so hs = win / 2 gives exact 50% Hann COLA.
    let hs = win / 2;
    let max_start = in_frames.saturating_sub(win);
    if max_start == 0 {
        return resample_catmull_rom(samples, ch, speed);
    }

    // 9 ms search radius -> 18 ms span covers fundamentals down to ~55 Hz while staying < hs/2.
    let delta_max = ((sample_rate as f32 * 0.009).round() as usize)
        .min(hs.saturating_sub(1))
        .max(1);

    let hann: Vec<f32> = (0..win)
        .map(|k| 0.5 * (1.0 - (2.0 * PI * k as f32 / win as f32).cos()))
        .collect();

    let buf_frames = out_frames + win;
    let mut out = vec![0.0_f32; buf_frames * ch];
    let mut weights = vec![0.0_f32; buf_frames];

    // Frame 0: flat leading half preserves attack transients without initial fade smear.
    for k in 0..win {
        let w = if k < hs { 1.0 } else { hann[k] };
        weights[k] += w;
        let src_base = k * ch;
        let dst_base = k * ch;
        for c in 0..ch {
            out[dst_base + c] += samples[src_base + c] * w;
        }
    }

    let score_lag = |cand_start: usize, ref_start: usize, nom_in: usize| -> Option<f64> {
        let mut dot = 0.0_f64;
        let mut cand_energy = 0.0_f64;
        let ref_slice = &samples[ref_start * ch..(ref_start + hs) * ch];
        let cand_slice = &samples[cand_start * ch..(cand_start + hs) * ch];
        for i in 0..ref_slice.len() {
            let r = f64::from(ref_slice[i]);
            let s = f64::from(cand_slice[i]);
            dot += r * s;
            cand_energy += s * s;
        }
        if cand_energy < 1e-12 {
            return None;
        }
        let corr = dot / cand_energy.sqrt();
        let dist = cand_start.abs_diff(nom_in) as f64 / delta_max as f64;
        Some(corr - 0.02 * dist * dist)
    };

    let ha_f = hs as f64 * speed as f64;
    let mut prev_in_pos = 0usize;
    let mut step_idx = 1usize;

    loop {
        let out_pos = step_idx * hs;
        if out_pos >= out_frames {
            break;
        }

        let nom_in = ((step_idx as f64 * ha_f).round() as usize).min(max_start);
        let ref_start = (prev_in_pos + hs).min(in_frames - hs);

        let ref_slice = &samples[ref_start * ch..(ref_start + hs) * ch];
        let ref_energy: f64 = ref_slice.iter().map(|&v| f64::from(v) * f64::from(v)).sum();

        let best_in = if ref_energy < 1e-12 {
            nom_in
        } else {
            let lo = nom_in.saturating_sub(delta_max);
            let hi = (nom_in + delta_max).min(max_start);

            // Coarse pass (step 2), then fine pass (+/- 2 around best coarse lag).
            let mut best_coarse = nom_in;
            let mut best_coarse_score = f64::NEG_INFINITY;
            let mut cand = lo;
            while cand <= hi {
                if let Some(score) = score_lag(cand, ref_start, nom_in) {
                    if score > best_coarse_score {
                        best_coarse_score = score;
                        best_coarse = cand;
                    }
                }
                cand += 2;
            }

            if best_coarse_score == f64::NEG_INFINITY {
                nom_in
            } else {
                let fine_lo = best_coarse.saturating_sub(2).max(lo);
                let fine_hi = (best_coarse + 2).min(hi);
                let mut best_fine = best_coarse;
                let mut best_fine_score = best_coarse_score;
                for f_cand in fine_lo..=fine_hi {
                    if let Some(score) = score_lag(f_cand, ref_start, nom_in) {
                        if score > best_fine_score {
                            best_fine_score = score;
                            best_fine = f_cand;
                        }
                    }
                }
                best_fine
            }
        };

        for k in 0..win {
            let w = hann[k];
            let dst_f = out_pos + k;
            weights[dst_f] += w;
            let src_base = (best_in + k) * ch;
            let dst_base = dst_f * ch;
            for c in 0..ch {
                out[dst_base + c] += samples[src_base + c] * w;
            }
        }

        prev_in_pos = best_in;
        step_idx += 1;
    }

    for f in 0..out_frames {
        let w = weights[f];
        if w > 1e-4 {
            let inv = 1.0 / w;
            let base = f * ch;
            for c in 0..ch {
                out[base + c] *= inv;
            }
        }
    }
    out.truncate(out_frames * ch);
    out
}

fn apply_stereo_freeverb(
    input: &AudioBuffer,
    wet: f32,
    room_size: f32,
    damping: f32,
) -> AudioBuffer {
    const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];
    const STEREO_SPREAD: usize = 23;
    const FIXED_GAIN: f32 = 0.015;

    let stereo = input.to_stereo();
    let sr = stereo.sample_rate.max(8_000);
    let scale = sr as f32 / 44_100.0;
    let dry_frames = stereo.samples.len() / 2;

    let tail_secs = (0.8 + room_size * 2.6).clamp(0.8, 3.6);
    let tail_frames = (tail_secs * sr as f32).round() as usize;
    let total_frames = dry_frames + tail_frames;

    let feedback = (0.70 + room_size * 0.27).clamp(0.68, 0.97);
    let damp = (damping * 0.55).clamp(0.0, 0.65);

    let mut combs_l: Vec<CombFilter> = COMB_TUNINGS
        .iter()
        .map(|&len| {
            let scaled = ((len as f32 * scale).round() as usize).max(8);
            CombFilter::new(scaled, feedback, damp)
        })
        .collect();
    let mut combs_r: Vec<CombFilter> = COMB_TUNINGS
        .iter()
        .map(|&len| {
            let scaled = (((len + STEREO_SPREAD) as f32 * scale).round() as usize).max(8);
            CombFilter::new(scaled, feedback, damp)
        })
        .collect();

    let mut allpass_l: Vec<AllpassFilter> = ALLPASS_TUNINGS
        .iter()
        .map(|&len| {
            let scaled = ((len as f32 * scale).round() as usize).max(4);
            AllpassFilter::new(scaled, 0.5)
        })
        .collect();
    let mut allpass_r: Vec<AllpassFilter> = ALLPASS_TUNINGS
        .iter()
        .map(|&len| {
            let scaled = (((len + STEREO_SPREAD) as f32 * scale).round() as usize).max(4);
            AllpassFilter::new(scaled, 0.5)
        })
        .collect();

    let dry_gain = (1.0 - wet * 0.55).clamp(0.0, 1.0);
    let wet_gain = wet * 1.15;

    let mut out = Vec::with_capacity(total_frames * 2);

    for f in 0..total_frames {
        let (dry_l, dry_r) = if f < dry_frames {
            (stereo.samples[f * 2], stereo.samples[f * 2 + 1])
        } else {
            (0.0, 0.0)
        };

        let mono_in = (dry_l + dry_r) * FIXED_GAIN;

        let mut wet_l = 0.0_f32;
        let mut wet_r = 0.0_f32;
        for c in 0..COMB_TUNINGS.len() {
            wet_l += combs_l[c].process(mono_in);
            wet_r += combs_r[c].process(mono_in);
        }
        for a in 0..ALLPASS_TUNINGS.len() {
            wet_l = allpass_l[a].process(wet_l);
            wet_r = allpass_r[a].process(wet_r);
        }

        out.push(dry_l * dry_gain + wet_l * wet_gain);
        out.push(dry_r * dry_gain + wet_r * wet_gain);
    }

    // Trim trailing silence below -60 dBFS after the original audio region, keeping a smooth fade-out.
    let min_keep = dry_frames + ((sr as f32 * 0.06) as usize).min(tail_frames);
    let mut last_audible = min_keep.min(total_frames);
    for f in (min_keep..total_frames).rev() {
        let peak = out[f * 2].abs().max(out[f * 2 + 1].abs());
        if peak >= 0.001 {
            last_audible = (f + (sr as usize / 25)).min(total_frames);
            break;
        }
    }
    out.truncate(last_audible * 2);

    // 20ms smooth tail fade-out prevents clicks at trimmed buffer boundary.
    let tail_fade_frames = ((sr as f32 * 0.02) as usize).min(last_audible);
    if tail_fade_frames > 0 {
        let start = last_audible - tail_fade_frames;
        for i in 0..tail_fade_frames {
            let frac = 1.0 - (i as f32 / tail_fade_frames as f32);
            let gain = 0.5 * (1.0 - (PI * frac).cos());
            let idx = (start + i) * 2;
            out[idx] *= gain;
            out[idx + 1] *= gain;
        }
    }

    AudioBuffer {
        samples: out,
        sample_rate: sr,
        channels: 2,
    }
}

fn apply_fades(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    fade_in_ms: u32,
    fade_out_ms: u32,
) {
    let total_frames = samples.len() / channels;
    if total_frames < 2 {
        return;
    }

    let mut in_frames = ((fade_in_ms as f32 * 0.001) * sample_rate as f32).round() as usize;
    let mut out_frames = ((fade_out_ms as f32 * 0.001) * sample_rate as f32).round() as usize;

    if in_frames + out_frames > total_frames {
        let scale = total_frames as f32 / (in_frames + out_frames) as f32;
        in_frames = (in_frames as f32 * scale) as usize;
        out_frames = total_frames.saturating_sub(in_frames);
    }

    if in_frames > 1 {
        for f in 0..in_frames {
            let t = f as f32 / in_frames as f32;
            let gain = 0.5 * (1.0 - (PI * t).cos());
            for c in 0..channels {
                samples[f * channels + c] *= gain;
            }
        }
    }

    if out_frames > 1 {
        let start = total_frames - out_frames;
        for i in 0..out_frames {
            let t = 1.0 - (i as f32 / out_frames as f32);
            let gain = 0.5 * (1.0 - (PI * t).cos());
            let f = start + i;
            for c in 0..channels {
                samples[f * channels + c] *= gain;
            }
        }
    }
}

fn apply_normalize_and_limiter(samples: &mut [f32], normalize: bool) {
    if normalize {
        let peak = samples.iter().fold(0.0_f32, |acc, &s| {
            if s.is_finite() {
                acc.max(s.abs())
            } else {
                acc
            }
        });
        if peak > 1e-6 {
            // -0.3 dBFS target = 10^(-0.3 / 20) = 0.96605
            let target_gain = 0.966_050_8 / peak;
            for s in samples.iter_mut() {
                *s *= target_gain;
            }
        }
    }

    // Soft-knee ceiling above 0.95 + hard safety clamp [-0.98, 0.98].
    for s in samples.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
            continue;
        }
        let abs = s.abs();
        if abs > 0.95 {
            let over = abs - 0.95;
            let shaped = 0.95 + 0.03 * (over / 0.03).tanh();
            *s = s.signum() * shaped;
        }
        *s = s.clamp(-0.98, 0.98);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sine_buffer(freq: f32, duration_secs: f32, sr: u32, channels: u16) -> AudioBuffer {
        let frames = (duration_secs * sr as f32) as usize;
        let mut samples = Vec::with_capacity(frames * channels as usize);
        for i in 0..frames {
            let t = i as f32 / sr as f32;
            let val = (t * freq * std::f32::consts::TAU).sin() * 0.5;
            for c in 0..channels {
                // Distinct slight offset on right channel to verify stereo ordering.
                let ch_val = if c == 0 { val } else { val * 0.5 };
                samples.push(ch_val);
            }
        }
        AudioBuffer {
            samples,
            sample_rate: sr,
            channels,
        }
    }

    #[test]
    fn reverse_preserves_stereo_channels() {
        let buf = AudioBuffer {
            samples: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
            sample_rate: 44_100,
            channels: 2,
        };
        let params = FxParams {
            reverse: true,
            normalize: false,
            ..Default::default()
        };
        let out = FxChain::process(&buf, &params);
        assert_eq!(out.channels, 2);
        assert_eq!(out.samples.len(), 6);
        assert!((out.samples[0] - 0.5).abs() < 1e-5);
        assert!((out.samples[1] - 0.6).abs() < 1e-5);
        assert!((out.samples[4] - 0.1).abs() < 1e-5);
        assert!((out.samples[5] - 0.2).abs() < 1e-5);
    }

    #[test]
    fn catmull_rom_slowed_and_speedup_lengths() {
        let buf = make_sine_buffer(220.0, 1.0, 44_100, 1);

        let slowed = FxChain::process(
            &buf,
            &FxParams {
                speed_pitch: 0.5,
                normalize: false,
                ..Default::default()
            },
        );
        assert!(
            (slowed.samples.len() as isize - 88_200).abs() <= 4,
            "expected ~2x length, got {}",
            slowed.samples.len()
        );

        let sped = FxChain::process(
            &buf,
            &FxParams {
                speed_pitch: 2.0,
                normalize: false,
                ..Default::default()
            },
        );
        assert!(
            (sped.samples.len() as isize - 22_050).abs() <= 4,
            "expected ~0.5x length, got {}",
            sped.samples.len()
        );
    }

    #[test]
    fn biquad_lowpass_attenuates_high_frequency() {
        let high_tone = make_sine_buffer(8_000.0, 0.5, 44_100, 1);
        let filtered = FxChain::process(
            &high_tone,
            &FxParams {
                lowpass_hz: 500.0,
                normalize: false,
                ..Default::default()
            },
        );
        let rms_in: f32 =
            (high_tone.samples.iter().map(|s| s * s).sum::<f32>() / high_tone.samples.len() as f32)
                .sqrt();
        let rms_out: f32 =
            (filtered.samples.iter().map(|s| s * s).sum::<f32>() / filtered.samples.len() as f32)
                .sqrt();
        assert!(
            rms_out < rms_in * 0.05,
            "expected strong LPF attenuation: in={rms_in}, out={rms_out}"
        );
    }

    #[test]
    fn freeverb_upmixes_mono_to_stereo_with_tail() {
        let mut impulse = vec![0.0_f32; 8_820]; // 0.2s mono
        impulse[100] = 0.8;
        let buf = AudioBuffer {
            samples: impulse,
            sample_rate: 44_100,
            channels: 1,
        };
        let out = FxChain::process(
            &buf,
            &FxParams {
                reverb_mix: 0.65,
                reverb_room_size: 0.85,
                reverb_damping: 0.3,
                normalize: true,
                ..Default::default()
            },
        );
        assert_eq!(out.channels, 2);
        assert!(
            out.samples.len() / 2 > 15_000,
            "expected reverb tail extension, got {} frames",
            out.samples.len() / 2
        );
    }

    #[test]
    fn drive_and_normalize_stays_within_limiter() {
        let buf = make_sine_buffer(110.0, 0.3, 44_100, 2);
        let out = FxChain::process(
            &buf,
            &FxParams {
                drive: 1.0,
                normalize: true,
                ..Default::default()
            },
        );
        assert!(out.samples.iter().all(|&s| s.is_finite() && s.abs() <= 0.98));
        let peak = out.samples.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.9, "expected normalized peak near -0.3 dBFS, got {peak}");
    }

    fn estimate_frequency_zero_crossings(samples: &[f32], sample_rate: u32) -> f32 {
        // Use the interior 60% to avoid boundary edges.
        let start = samples.len() / 5;
        let end = (samples.len() * 4) / 5;
        let slice = &samples[start..end];
        let mut first_idx = None;
        let mut crossings = 0usize;
        for i in 1..slice.len() {
            if slice[i - 1] <= 0.0 && slice[i] > 0.0 {
                // Linear interpolation for sub-sample crossing accuracy.
                let frac = -slice[i - 1] / (slice[i] - slice[i - 1]);
                let pos = i as f32 - 1.0 + frac;
                if first_idx.is_none() {
                    first_idx = Some(pos);
                } else {
                    crossings += 1;
                }
            }
        }
        let Some(first) = first_idx else {
            return 0.0;
        };
        let mut last = first;
        for i in (1..slice.len()).rev() {
            if slice[i - 1] <= 0.0 && slice[i] > 0.0 {
                let frac = -slice[i - 1] / (slice[i] - slice[i - 1]);
                last = i as f32 - 1.0 + frac;
                break;
            }
        }
        let span_secs = (last - first) / sample_rate as f32;
        if span_secs <= 0.0 || crossings == 0 {
            return 0.0;
        }
        crossings as f32 / span_secs
    }

    #[test]
    fn wsola_tempo_only_lengths_and_no_clipping() {
        let sr = 44_100;
        let buf = make_sine_buffer(330.0, 1.0, sr, 2);
        let in_frames = buf.samples.len() / 2;

        for speed in [0.5_f32, 0.75, 1.25, 1.5, 2.0] {
            for normalize in [false, true] {
                let out = FxChain::process(
                    &buf,
                    &FxParams {
                        speed_pitch: speed,
                        speed_mode: SpeedMode::TempoOnly,
                        normalize,
                        ..Default::default()
                    },
                );
                let expected = in_frames as f32 / speed;
                let out_frames = out.samples.len() / 2;
                let rel_err = (out_frames as f32 - expected).abs() / expected;
                assert!(
                    rel_err <= 0.02,
                    "speed {speed}: expected ~{expected}, got {out_frames} (err {rel_err})"
                );
                assert!(
                    out.samples.iter().all(|&s| s.is_finite() && s.abs() <= 0.98),
                    "speed {speed} normalize {normalize}: clipping or non-finite sample detected"
                );
            }
        }
    }

    #[test]
    fn wsola_preserves_stereo_channel_sync() {
        let sr = 44_100;
        let mut buf = make_sine_buffer(220.0, 1.0, sr, 2);
        // Add a synchronized amplitude burst in the middle so timing sync can also be checked.
        for f in 20_000..22_000 {
            buf.samples[f * 2] *= 1.8;
            buf.samples[f * 2 + 1] *= 1.8;
        }

        for speed in [0.7_f32, 1.5] {
            let out = FxChain::process(
                &buf,
                &FxParams {
                    speed_pitch: speed,
                    speed_mode: SpeedMode::TempoOnly,
                    normalize: false,
                    ..Default::default()
                },
            );
            assert_eq!(out.channels, 2);
            let mut max_l = (0usize, 0.0_f32);
            let mut max_r = (0usize, 0.0_f32);
            for (f, frame) in out.samples.chunks_exact(2).enumerate() {
                assert!(
                    (frame[1] - frame[0] * 0.5).abs() < 1e-4,
                    "stereo proportionality drifted at frame {f}: L={}, R={}",
                    frame[0],
                    frame[1]
                );
                if frame[0].abs() > max_l.1 {
                    max_l = (f, frame[0].abs());
                }
                if frame[1].abs() > max_r.1 {
                    max_r = (f, frame[1].abs());
                }
            }
            assert_eq!(max_l.0, max_r.0, "L/R peak frames diverged at speed {speed}");
        }
    }

    #[test]
    fn wsola_preserves_sine_pitch() {
        let sr = 44_100;
        let freq = 440.0_f32;
        let buf = make_sine_buffer(freq, 1.0, sr, 1);

        for speed in [0.7_f32, 1.5] {
            let out = FxChain::process(
                &buf,
                &FxParams {
                    speed_pitch: speed,
                    speed_mode: SpeedMode::TempoOnly,
                    normalize: false,
                    ..Default::default()
                },
            );
            let measured = estimate_frequency_zero_crossings(&out.samples, sr);
            let rel_err = (measured - freq).abs() / freq;
            assert!(
                rel_err <= 0.03,
                "speed {speed}: expected pitch ~{freq} Hz, measured {measured} Hz"
            );
        }

        // Sanity check: PitchAndTempo at 1.5x actually shifts pitch to ~660 Hz.
        let pitched = FxChain::process(
            &buf,
            &FxParams {
                speed_pitch: 1.5,
                speed_mode: SpeedMode::PitchAndTempo,
                normalize: false,
                ..Default::default()
            },
        );
        let pitched_freq = estimate_frequency_zero_crossings(&pitched.samples, sr);
        assert!(
            (pitched_freq - 660.0).abs() < 15.0,
            "expected ~660 Hz in PitchAndTempo, got {pitched_freq}"
        );
    }

    #[test]
    fn speed_mode_changes_filename_suffix_and_clean_state() {
        let clean = FxParams::default();
        assert!(clean.is_fx_clean());
        assert!(clean.filename_suffix().is_empty());

        let pitch_slow = FxParams {
            speed_pitch: 0.8,
            speed_mode: SpeedMode::PitchAndTempo,
            ..Default::default()
        };
        let tempo_slow = FxParams {
            speed_pitch: 0.8,
            speed_mode: SpeedMode::TempoOnly,
            ..Default::default()
        };
        assert!(!tempo_slow.is_fx_clean());
        assert_ne!(pitch_slow.filename_suffix(), tempo_slow.filename_suffix());
        assert!(tempo_slow.filename_suffix().contains("slowed_tempo_"));

        let tempo_fast = FxParams {
            speed_pitch: 1.4,
            speed_mode: SpeedMode::TempoOnly,
            ..Default::default()
        };
        assert!(tempo_fast.filename_suffix().contains("speedup_tempo_"));
    }
}
