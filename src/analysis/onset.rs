//! Detección de onsets: flujo espectral + umbral adaptativo.
//!
//! El flujo espectral mide cuánto CRECE el espectro entre frames sucesivos
//! (solo incrementos positivos: los ataques suman, las colas no restan). El
//! umbral adaptativo compara contra la media móvil local + margen, de modo que
//! funciona igual con música tranquila y con mezclas densas.

use std::collections::VecDeque;

/// Flujo espectral de `mags` frente al frame anterior (normalizado por nº de
/// bins → rango típico 0..~1).
#[derive(Debug, Default)]
pub struct FluxAnalyzer {
    /// Espectro anterior REUTILIZADO (dos buffers alternados: cero allocs).
    prev: Vec<f32>,
    primed: bool,
}

impl FluxAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn flux(&mut self, mags: &[f32]) -> f32 {
        if !self.primed || self.prev.len() != mags.len() {
            self.prev.clear();
            self.prev.extend_from_slice(mags);
            self.primed = true;
            return 0.0;
        }
        let mut sum = 0.0f32;
        for (m, p) in mags.iter().zip(self.prev.iter()) {
            let d = m - p;
            if d > 0.0 {
                sum += d;
            }
        }
        // Copia en sitio sobre el buffer ya retenido.
        self.prev.copy_from_slice(mags);
        sum / mags.len().max(1) as f32
    }
}

/// Resultado de un frame para el detector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnsetOutcome {
    /// Fuerza normalizada 0..1 del candidato a onset en ESTE frame.
    pub strength: f32,
    /// `true` cuando se cruza el umbral adaptativo (impulso discreto).
    pub triggered: bool,
}

/// Detector con umbral adaptativo sobre el historial del flujo.
#[derive(Debug)]
pub struct OnsetDetector {
    history: VecDeque<f32>,
    window: usize,
    /// Margen sobre la media local (en unidades de flujo).
    delta: f32,
}

impl OnsetDetector {
    /// `window` = frames del promedio local (~0.5 s al hop típico).
    pub fn new(window: usize, delta: f32) -> Self {
        Self {
            history: VecDeque::with_capacity(window + 1),
            window: window.max(1),
            delta,
        }
    }

    pub fn observe(&mut self, flux: f32) -> OnsetOutcome {
        self.history.push_back(flux);
        while self.history.len() > self.window {
            self.history.pop_front();
        }
        if self.history.len() < self.window / 2 {
            // Sin historia suficiente: calibrando, nunca disparar.
            return OnsetOutcome {
                strength: 0.0,
                triggered: false,
            };
        }
        let mean = self.history.iter().sum::<f32>() / self.history.len() as f32;
        let excess = flux - mean - self.delta;
        if excess <= 0.0 {
            return OnsetOutcome {
                strength: 0.0,
                triggered: false,
            };
        }
        // Normalización robusta: escala = media local + delta (el propio
        // umbral). Un pico del doble del umbral satura hacia 1.
        let scale = (mean + self.delta).max(1e-6);
        let strength = (excess / scale).min(1.0);
        OnsetOutcome {
            strength,
            triggered: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flux_is_zero_for_constant_spectrum_and_first_frame() {
        let mut f = FluxAnalyzer::new();
        let mags = vec![0.5f32; 64];
        assert_eq!(f.flux(&mags), 0.0, "primer frame sin referencia");
        assert_eq!(f.flux(&mags), 0.0, "espectro constante");
    }

    #[test]
    fn flux_only_counts_growth() {
        let mut f = FluxAnalyzer::new();
        let quiet = vec![0.1f32; 16];
        let loud = vec![0.9f32; 16];
        f.flux(&quiet);
        let up = f.flux(&loud); // subida: suma
        assert!(up > 0.5);
        let down = f.flux(&quiet); // bajada: NO resta
        assert_eq!(down, 0.0);
    }

    #[test]
    fn impulses_trigger_and_steady_tone_does_not() {
        let mut det = OnsetDetector::new(43, 0.005);

        // Tono estable: el flujo se estanca cerca de cero tras el arranque.
        let mut fired_during_steady = false;
        for _ in 0..100 {
            let o = det.observe(0.002);
            if o.triggered {
                fired_during_steady = true;
            }
        }
        assert!(!fired_during_steady, "un tono constante no dispara onsets");

        // Impulso: pico muy superior al umbral adaptativo.
        let o = det.observe(0.35);
        assert!(o.triggered && o.strength > 0.3, "el impulso dispara: {o:?}");

        // Vuelve la calma: nada más dispara.
        for _ in 0..50 {
            assert!(!det.observe(0.002).triggered);
        }
    }

    #[test]
    fn regular_pulse_train_triggers_per_impulse() {
        let mut det = OnsetDetector::new(43, 0.005);
        let mut hits = 0;
        for i in 0..400 {
            let flux = if i % 40 == 0 { 0.30 } else { 0.001 };
            if det.observe(flux).triggered {
                hits += 1;
            }
        }
        assert!(
            (8..=12).contains(&hits),
            "400 frames con pulso cada 40 → ~10 onsets, hubo {hits}"
        );
    }
}
