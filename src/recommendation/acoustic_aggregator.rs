//! Agregador acústico (FASE 8): reduce los frames de `AudioFeatures` en vivo a
//! un `TrackAcousticProfile` promedio persistible.
//!
//! El análisis de audio es EFÍMERO (solo existe durante la reproducción). Para
//! que el sistema de recomendaciones pueda comparar tracks por su sonido, este
//! agregador acumula los frames durante una reproducción y, al terminar, produce
//! un perfil que se guarda en `track_acoustic_profiles`.

use crate::analysis::features::AudioFeatures;
use crate::recommendation::types::TrackAcousticProfile;

/// Acumula frames de `AudioFeatures` de un track para producir su perfil medio.
#[derive(Debug, Default, Clone)]
pub struct AcousticAggregator {
    track_id: i64,
    frames: u64,
    rms: f64,
    bass: f64,
    low_mid: f64,
    mid: f64,
    high_mid: f64,
    high: f64,
    centroid: f64,
    onset: f64,
    bpm_sum: f64,
    bpm_sq: f64,
}

impl AcousticAggregator {
    pub fn new(track_id: i64) -> Self {
        Self {
            track_id,
            ..Self::default()
        }
    }

    /// Añade un frame. Ignora frames silenciosos (todos a cero) que todavía no
    /// codifican sonido, y evita que un `bpm=0` "desconocido" arrastre la media.
    pub fn add(&mut self, f: &AudioFeatures) {
        let active = f.rms > 0.0
            || f.bass + f.low_mid + f.mid + f.high_mid + f.high > 0.0
            || f.spectral_centroid > 0.0;
        if !active {
            return;
        }
        self.frames += 1;
        self.rms += f.rms as f64;
        self.bass += f.bass as f64;
        self.low_mid += f.low_mid as f64;
        self.mid += f.mid as f64;
        self.high_mid += f.high_mid as f64;
        self.high += f.high as f64;
        self.centroid += f.spectral_centroid as f64;
        self.onset += f.onset as f64;
        if f.bpm > 0.0 {
            self.bpm_sum += f.bpm as f64;
            self.bpm_sq += (f.bpm as f64) * (f.bpm as f64);
        }
    }

    /// ¿Hay suficientes frames como para emitir un perfil mínimamente fiable?
    pub fn ready(&self) -> bool {
        self.frames >= 30
    }

    fn mean(&self, sum: f64) -> f32 {
        if self.frames == 0 {
            0.0
        } else {
            (sum / self.frames as f64) as f32
        }
    }

    /// Produce el perfil promedio. Devuelve `None` si no hubo frames útiles o
    /// si aún no hay suficientes para un perfil fiable.
    pub fn into_profile(self) -> Option<TrackAcousticProfile> {
        if !self.ready() {
            return None;
        }
        let frames = self.frames;
        let bpm_mean = self.mean(self.bpm_sum) as f64;
        // Varianza de BPM solo sobre frames con BPM conocido y medido.
        let bpm_variance = {
            let n = self.frames.max(1) as f64;
            let raw = (self.bpm_sq / n - bpm_mean * bpm_mean).max(0.0);
            raw as f32
        };
        Some(TrackAcousticProfile {
            track_id: self.track_id,
            rms_mean: self.mean(self.rms),
            bass_mean: self.mean(self.bass),
            low_mid_mean: self.mean(self.low_mid),
            mid_mean: self.mean(self.mid),
            high_mid_mean: self.mean(self.high_mid),
            high_mean: self.mean(self.high),
            spectral_centroid_mean: self.mean(self.centroid),
            bpm_mean: bpm_mean as f32,
            bpm_variance,
            onset_mean: self.mean(self.onset),
            band_profile: [
                self.mean(self.bass),
                self.mean(self.low_mid),
                self.mean(self.mid),
                self.mean(self.high_mid),
                self.mean(self.high),
            ],
            frame_count: frames as i64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::features::AudioFeatures;

    fn frame(bass: f32, mid: f32, high: f32, bpm: f32) -> AudioFeatures {
        AudioFeatures {
            timestamp: std::time::Duration::ZERO,
            rms: 0.2,
            amplitude: 0.3,
            bass,
            low_mid: 0.0,
            mid,
            high_mid: 0.0,
            high,
            spectral_centroid: 0.5,
            spectral_flux: 0.1,
            onset: 0.0,
            beat: false,
            beat_confidence: 0.0,
            bpm,
        }
    }

    #[test]
    fn aggregates_mean_per_band() {
        let mut agg = AcousticAggregator::new(7);
        for _ in 0..100 {
            agg.add(&frame(0.1, 0.2, 0.3, 120.0));
        }
        let p = agg.into_profile().unwrap();
        assert_eq!(p.track_id, 7);
        assert_eq!(p.frame_count, 100);
        assert!((p.bass_mean - 0.1).abs() < 1e-4);
        assert!((p.mid_mean - 0.2).abs() < 1e-4);
        assert!((p.high_mean - 0.3).abs() < 1e-4);
        assert!((p.bpm_mean - 120.0).abs() < 1e-3);
    }

    #[test]
    fn not_ready_with_few_frames() {
        let mut agg = AcousticAggregator::new(1);
        for _ in 0..5 {
            agg.add(&frame(0.1, 0.1, 0.1, 100.0));
        }
        assert!(!agg.ready());
        assert!(agg.into_profile().is_none());
    }

    #[test]
    fn ignores_silent_frames() {
        let mut agg = AcousticAggregator::new(1);
        for _ in 0..50 {
            agg.add(&AudioFeatures::silent(std::time::Duration::ZERO));
        }
        assert!(!agg.ready(), "frames silenciosos no cuentan");
    }
}
