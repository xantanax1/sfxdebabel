use bytes::Bytes;
use rayon::prelude::*;

use crate::audio::decoder::{decode_audio_bytes, DecodedAudio};
use crate::error::AppError;

pub const ENVELOPE_POINTS: usize = 64;

/// Computes a 64-point Root Mean Square (RMS) amplitude envelope normalized to [0.0, 1.0].
pub fn compute_rms_envelope(mono_samples: &[f32]) -> Vec<f32> {
    if mono_samples.is_empty() {
        return vec![0.0; ENVELOPE_POINTS];
    }

    let total = mono_samples.len();
    let mut bins = Vec::with_capacity(ENVELOPE_POINTS);

    // Partition samples evenly across 64 windows even when sample count < 64.
    for i in 0..ENVELOPE_POINTS {
        let start = (i * total) / ENVELOPE_POINTS;
        let mut end = ((i + 1) * total) / ENVELOPE_POINTS;
        if end <= start {
            end = (start + 1).min(total);
        }
        let window = &mono_samples[start..end];
        let mean_sq = window.iter().map(|&s| s * s).sum::<f32>() / window.len() as f32;
        bins.push(mean_sq.sqrt());
    }

    let peak = bins.iter().copied().fold(0.0_f32, f32::max);
    if peak > 1e-7 {
        for v in &mut bins {
            *v = (*v / peak).clamp(0.0, 1.0);
        }
    } else {
        bins.fill(0.0);
    }

    bins
}

/// Resamples (if needed) and peak-normalizes a user-drawn envelope curve into 64 points in [0.0, 1.0].
pub fn normalize_envelope(curve: &[f32]) -> Vec<f32> {
    if curve.is_empty() {
        return vec![0.0; ENVELOPE_POINTS];
    }

    let mut out = Vec::with_capacity(ENVELOPE_POINTS);
    if curve.len() == ENVELOPE_POINTS {
        for &v in curve {
            out.push(if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
        }
    } else if curve.len() == 1 {
        let v = if curve[0].is_finite() {
            curve[0].clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.resize(ENVELOPE_POINTS, v);
    } else {
        let max_idx = (curve.len() - 1) as f32;
        for i in 0..ENVELOPE_POINTS {
            let pos = (i as f32 / (ENVELOPE_POINTS - 1) as f32) * max_idx;
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(curve.len() - 1);
            let frac = pos - lo as f32;
            let a = if curve[lo].is_finite() {
                curve[lo].clamp(0.0, 1.0)
            } else {
                0.0
            };
            let b = if curve[hi].is_finite() {
                curve[hi].clamp(0.0, 1.0)
            } else {
                0.0
            };
            out.push(a + (b - a) * frac);
        }
    }

    let peak = out.iter().copied().fold(0.0_f32, f32::max);
    if peak > 1e-6 {
        for v in &mut out {
            *v = (*v / peak).clamp(0.0, 1.0);
        }
    }
    out
}

/// Computes perceptual envelope similarity percentage [0.0, 100.0] by combining
/// directional shape alignment (cosine similarity) and magnitude closeness (1 - RMSE)
/// across small +/-2-bin shifts to tolerate MP3 encoder padding / leading silence.
#[allow(dead_code)] // default-weight shorthand, used by tests
pub fn compute_similarity(target: &[f32], candidate: &[f32]) -> f32 {
    compute_similarity_weighted(target, candidate, DEFAULT_COSINE_WEIGHT)
}

/// Default share of the cosine (shape) term; the rest goes to the RMSE (level) term.
pub const DEFAULT_COSINE_WEIGHT: f32 = 0.65;

/// Same metric as [`compute_similarity`] with a configurable blend:
/// `score = w * cosine + (1 - w) * (1 - rmse)`, `w` clamped to `[0, 1]`.
pub fn compute_similarity_weighted(target: &[f32], candidate: &[f32], cosine_weight: f32) -> f32 {
    let w = if cosine_weight.is_finite() {
        cosine_weight.clamp(0.0, 1.0)
    } else {
        DEFAULT_COSINE_WEIGHT
    };
    let a = normalize_envelope(target);
    let b = normalize_envelope(candidate);

    let energy_a: f32 = a.iter().map(|v| v * v).sum();
    let energy_b: f32 = b.iter().map(|v| v * v).sum();
    if energy_a < 1e-7 || energy_b < 1e-7 {
        return 0.0;
    }

    let mut best_score = 0.0_f32;
    let n = ENVELOPE_POINTS as isize;

    for shift in -2_isize..=2_isize {
        let mut dot = 0.0_f32;
        let mut norm_a = 0.0_f32;
        let mut norm_b = 0.0_f32;
        let mut sq_err = 0.0_f32;

        for i in 0..n {
            let va = a[i as usize];
            let j = i + shift;
            let vb = if (0..n).contains(&j) {
                b[j as usize]
            } else {
                0.0
            };
            dot += va * vb;
            norm_a += va * va;
            norm_b += vb * vb;
            let diff = va - vb;
            sq_err += diff * diff;
        }

        let denom = norm_a.sqrt() * norm_b.sqrt();
        let cosine = if denom > 1e-7 {
            (dot / denom).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rmse = (sq_err / ENVELOPE_POINTS as f32).sqrt().clamp(0.0, 1.0);
        let combined = (w * cosine + (1.0 - w) * (1.0 - rmse)).clamp(0.0, 1.0);
        if combined > best_score {
            best_score = combined;
        }
    }

    (best_score * 100.0 * 10.0).round() / 10.0
}

/// Decodes raw audio payloads and computes 64-point RMS envelopes in parallel via Rayon.
pub fn process_batch_parallel(
    raw_items: Vec<(String, Bytes, Option<String>)>,
) -> Vec<(String, Result<(DecodedAudio, Vec<f32>), AppError>)> {
    raw_items
        .into_par_iter()
        .map(|(id, bytes, ext_hint)| {
            let res = decode_audio_bytes(bytes, ext_hint.as_deref()).map(|decoded| {
                let env = compute_rms_envelope(&decoded.mono_samples);
                (decoded, env)
            });
            (id, res)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_envelope_has_64_normalized_points() {
        let mut samples = Vec::with_capacity(44_100);
        for i in 0..44_100 {
            let t = i as f32 / 44_100.0;
            samples.push((t * 440.0 * std::f32::consts::TAU).sin() * t);
        }
        let env = compute_rms_envelope(&samples);
        assert_eq!(env.len(), ENVELOPE_POINTS);
        assert!(env.iter().all(|&v| (0.0..=1.0).contains(&v)));
        assert!(env.last().copied().unwrap_or(0.0) > 0.9);
        assert!(env.first().copied().unwrap_or(1.0) < 0.1);
    }

    #[test]
    fn similarity_distinguishes_impulse_from_riser() {
        let mut impulse = vec![0.02_f32; ENVELOPE_POINTS];
        impulse[0] = 1.0;
        impulse[1] = 0.85;
        impulse[2] = 0.55;
        impulse[3] = 0.30;

        let riser: Vec<f32> = (0..ENVELOPE_POINTS)
            .map(|i| (i as f32 / (ENVELOPE_POINTS - 1) as f32).powi(2))
            .collect();

        let self_score = compute_similarity(&impulse, &impulse);
        let cross_score = compute_similarity(&impulse, &riser);

        assert!(self_score > 99.5);
        assert!(cross_score < 45.0, "expected low score, got {cross_score}");
    }

    #[test]
    fn similarity_tolerates_small_lag_shift() {
        let mut a = vec![0.0_f32; ENVELOPE_POINTS];
        let mut b = vec![0.0_f32; ENVELOPE_POINTS];
        for i in 0..ENVELOPE_POINTS {
            let x = (i as f32 - 24.0) / 7.0;
            a[i] = (-0.5 * x * x).exp();
            let x_shifted = (i as f32 - 26.0) / 7.0;
            b[i] = (-0.5 * x_shifted * x_shifted).exp();
        }
        let score = compute_similarity(&a, &b);
        assert!(score > 95.0, "expected >95% under 2-bin shift, got {score}");
    }

    #[test]
    fn weight_shifts_between_shape_and_level() {
        // Same shape, different level floor: cosine stays high, RMSE penalizes.
        let a: Vec<f32> = (0..ENVELOPE_POINTS)
            .map(|i| if i < 32 { 1.0 } else { 0.0 })
            .collect();
        let b: Vec<f32> = (0..ENVELOPE_POINTS)
            .map(|i| if i < 32 { 1.0 } else { 0.35 })
            .collect();
        let shape_only = compute_similarity_weighted(&a, &b, 1.0);
        let level_only = compute_similarity_weighted(&a, &b, 0.0);
        let default = compute_similarity(&a, &b);
        assert!(shape_only > level_only, "{shape_only} vs {level_only}");
        assert!(default >= level_only.min(shape_only) && default <= level_only.max(shape_only));
        // Out-of-range / NaN weights are sanitized.
        assert_eq!(
            compute_similarity_weighted(&a, &b, 7.0),
            compute_similarity_weighted(&a, &b, 1.0)
        );
        assert_eq!(compute_similarity_weighted(&a, &b, f32::NAN), default);
    }

    #[test]
    fn empty_input_is_safe() {
        let env = compute_rms_envelope(&[]);
        assert_eq!(env.len(), ENVELOPE_POINTS);
        assert!(env.iter().all(|&v| v == 0.0));
        assert_eq!(compute_similarity(&env, &env), 0.0);
    }
}
