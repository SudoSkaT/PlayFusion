//! Medidas de amplitud en dominio temporal: RMS y pico.

/// Raíz del promedio cuadrático de `samples` (0 para entrada vacía).
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Amplitud pico (máximo |valor|).
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |a, &s| a.max(s.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_constant_equals_amplitude() {
        assert!((rms(&[0.5; 100]) - 0.5).abs() < 1e-6);
        assert!((rms(&[-0.25; 64]) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn silence_is_zero_and_empty_is_zero() {
        assert_eq!(rms(&[0.0; 128]), 0.0);
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(peak(&[]), 0.0);
    }

    #[test]
    fn sine_rms_matches_theory() {
        // RMS de un seno = A/√2.
        let n = 4096;
        let a = 0.8f32;
        let sine: Vec<f32> = (0..n)
            .map(|i| a * (2.0 * std::f32::consts::PI * i as f32 * 10.0 / n as f32).sin())
            .collect();
        assert!((rms(&sine) - a / std::f32::consts::SQRT_2).abs() < 1e-3);
        assert!((peak(&sine) - a).abs() < 1e-3);
    }

    #[test]
    fn ramp_tracks_amplitude_growth() {
        let quiet = vec![0.05f32; 100];
        let loud = vec![0.6f32; 100];
        assert!(rms(&loud) > rms(&quiet) * 5.0);
    }
}
