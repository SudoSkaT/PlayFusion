//! Estimación de tempo (BPM) y confianza de beat por autocorrelación del
//! envelope de onsets.
//!
//! Método clásico y barato: el envelope de fuerzas de onset a hop-rate fijo
//! se autocorrela SOLO en los lags correspondientes al rango musical
//! 60–200 BPM; el lag con máxima correlación da el tempo y la relación entre
//! esa correlación y la energía total da la confianza. Coste por recálculo:
//! ~140 lags × ventana de ~4 s ≈ decenas de miles de multiplicaciones, cada
//! pocos frames — despreciable frente a la FFT.

use std::collections::VecDeque;

/// Estimación de tempo para un frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoEstimate {
    /// BPM estimado (0 = sin señal suficiente).
    pub bpm: f32,
    /// Confianza 0..1: pico de autocorrelación normalizado.
    pub confidence: f32,
}

/// Estimador sobre el historial del envelope de onsets.
#[derive(Debug)]
pub struct BpmEstimator {
    envelope: VecDeque<f32>,
    capacity: usize,
    hop_rate: f32,
    min_bpm: f32,
    max_bpm: f32,
    /// Recalcula cada N pushes (coste acotado).
    recalc_every: usize,
    since_recalc: usize,
    last: TempoEstimate,
}

impl BpmEstimator {
    /// `hop_rate` = frames de envelope por segundo (p. ej. 86 fps con
    /// fft=2048/hop=512 @44.1 kHz).
    pub fn new(hop_rate: f32) -> Self {
        let window_secs = 4.0;
        Self {
            envelope: VecDeque::with_capacity((hop_rate * window_secs) as usize),
            capacity: (hop_rate * window_secs) as usize,
            hop_rate,
            min_bpm: 60.0,
            max_bpm: 200.0,
            recalc_every: 8,
            since_recalc: 0,
            last: TempoEstimate { bpm: 0.0, confidence: 0.0 },
        }
    }

    /// Frames de historia acumulados (tests/observabilidad).
    pub fn history_len(&self) -> usize {
        self.envelope.len()
    }

    /// Incorpora un valor de envelope y devuelve el estimate vigente.
    pub fn observe(&mut self, strength: f32) -> TempoEstimate {
        self.envelope.push_back(strength);
        if self.envelope.len() > self.capacity {
            self.envelope.pop_front();
        }
        self.since_recalc += 1;
        if self.since_recalc >= self.recalc_every || self.last.bpm == 0.0 {
            self.since_recalc = 0;
            self.last = self.estimate();
        }
        self.last
    }

    pub fn last(&self) -> TempoEstimate {
        self.last
    }

    /// Autocorrelación restringida al rango de BPM musical.
    pub fn estimate(&self) -> TempoEstimate {
        let n = self.envelope.len();
        // Mínimo ~1.5 s de historia y señal real: sin esto, cualquier ruido
        // produciría tempos fantasiosos.
        if n < (self.hop_rate * 1.5) as usize {
            return TempoEstimate { bpm: 0.0, confidence: 0.0 };
        }
        let energy: f32 = self.envelope.iter().map(|v| v * v).sum();
        if energy < 1e-4 {
            return TempoEstimate { bpm: 0.0, confidence: 0.0 };
        }

        let lag_min = ((self.hop_rate * 60.0 / self.max_bpm).floor() as usize).max(1);
        let lag_max = ((self.hop_rate * 60.0 / self.min_bpm).ceil() as usize).min(n / 2);

        let mut best_corr = 0.0f32;
        let mut best_lag = 0usize;
        for lag in lag_min..=lag_max.max(lag_min + 1) {
            let mut acc = 0.0f32;
            for i in 0..(n - lag) {
                acc += self.envelope[i] * self.envelope[i + lag];
            }
            let corr = acc / (n - lag) as f32;
            if corr > best_corr {
                best_corr = corr;
                best_lag = lag;
            }
        }
        if best_lag == 0 {
            return TempoEstimate { bpm: 0.0, confidence: 0.0 };
        }

        let confidence = (best_corr * n as f32 / energy).clamp(0.0, 1.0);
        let bpm = 60.0 * self.hop_rate / best_lag as f32;

        // Octava-error clásico: si el doble cabe en rango con soporte similar,
        // preferir el más rápido es arriesgado; aquí se deja el crudo — el
        // refinado fino no aporta al objetivo visual actual (spec §20).
        TempoEstimate { bpm, confidence }
    }
}

#[cfg(test)]
mod tests {

use super::*;

    const RATE: f32 = 86.13; // fps del hop 512 @44.1k

    #[test]
    fn click_track_at_120bpm_is_detected_with_confidence() {
        let mut est = BpmEstimator::new(RATE);
        // Pulso cada 0.5 s → 43 frames de hop.
        let period = (RATE * 0.5).round() as usize;
        let mut out = TempoEstimate { bpm: 0.0, confidence: 0.0 };
        for i in 0..(RATE as usize * 5) {
            let strength = if i % period == 0 { 1.0 } else { 0.0 };
            out = est.observe(strength);
        }
        assert!(
            (out.bpm - 120.0).abs() < 6.0,
            "BPM detectado {} ≈ 120",
            out.bpm
        );
        assert!(out.confidence > 0.6, "confianza alta: {}", out.confidence);
    }

    #[test]
    fn silence_and_noise_give_no_tempo() {
        let mut est = BpmEstimator::new(RATE);
        for _ in 0..(RATE as usize * 3) {
            est.observe(0.0);
        }
        assert_eq!(est.last().bpm, 0.0);

        let mut noisy = BpmEstimator::new(RATE);
        // Ruido decorrelado (hash sinusoidal): sin pulso periódico ⇒ la
        // autocorrelación no debe sostener confianza alta sostenida.
        let mut high_conf_frames = 0usize;
        for i in 0..(RATE as usize * 4) {
            let v = ((i as f32 * 12.9898).sin() * 43_758.55).fract().abs();
            let t = noisy.observe(v);
            if t.confidence >= 0.85 {
                high_conf_frames += 1;
            }
        }
        assert!(
            high_conf_frames <= RATE as usize,
            "un ruido sin pulso apenas alcanza confianza extrema ({high_conf_frames} frames)"
        );
    }

    #[test]
    fn short_history_returns_zero() {
        let mut est = BpmEstimator::new(RATE);
        for _ in 0..20 {
            est.observe(1.0);
        }
        assert_eq!(est.last().bpm, 0.0, "menos de 1.5s no estima");
    }
}
