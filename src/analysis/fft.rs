//! FFT del pipeline de análisis: ventana de Hann + espectro de magnitudes.
//!
//! Usa `rustfft` (MIT/Apache-2.0): plan cacheado y scratch reutilizados — el
//! camino caliente NO aloja por frame (spec §34). Entrada real, salida: la
//! primera mitad del espectro (`size/2` bins), suficiente para features.

use rustfft::{num_complex::Complex, FftPlanner};

/// Analizador espectral para un tamaño de ventana fijo.
pub struct SpectrumAnalyzer {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Buffer complejo reutilizado entrada tras entrada (cero allocs/frame).
    cplx: Vec<Complex<f32>>,
    window: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(fft_size: usize) -> Self {
        assert!(fft_size.is_power_of_two(), "fft_size debe ser potencia de 2");
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        let window: Vec<f32> = hann_window(fft_size);
        Self {
            fft,
            scratch,
            cplx: Vec::with_capacity(fft_size),
            window,
        }
    }

    pub fn fft_size(&self) -> usize {
        self.fft.len()
    }

    /// Ventana de Hann precalculada (para inspección/tests).
    pub fn window(&self) -> &[f32] {
        &self.window
    }

    /// Espectro de magnitudes de `samples` (debe medir `fft_size`).
    ///
    /// Devuelve `fft_size/2` valores ≥ 0 (bins 0..nyquist). Versión cómoda
    /// que aloca; el camino caliente usa [`Self::magnitudes_into`].
    pub fn magnitudes(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.fft.len() / 2);
        self.magnitudes_into(samples, &mut out);
        out
    }

    /// [`Self::magnitudes`] SIN allocation por llamada: reutiliza `out`
    /// (se trunca/rellena; la capacidad queda retenida entre frames).
    pub fn magnitudes_into(&mut self, samples: &[f32], out: &mut Vec<f32>) {
        debug_assert_eq!(samples.len(), self.window.len());
        let n = self.window.len();
        // El buffer complejo sí se reutiliza entre llamadas del mismo analyzer.
        self.cplx.clear();
        self.cplx.extend(
            samples.iter().zip(&self.window).map(|(&s, &w)| Complex::new(s * w, 0.0)),
        );
        self.fft.process_with_scratch(&mut self.cplx, &mut self.scratch);
        out.clear();
        out.extend(self.cplx[..n / 2].iter().map(|c| c.norm()));
    }
}

/// Ventana de Hann periódica-equivalente (denominador N-1): extremos ~0,
/// simétrica, suma conocida.
pub fn hann_window(size: usize) -> Vec<f32> {
    if size <= 1 {
        return vec![1.0; size];
    }
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (size - 1) as f64).cos()) as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn sine(freq: f32, secs: f32, amp: f32) -> Vec<f32> {
        let n = (SR * secs) as usize;
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn hann_is_symmetric_and_fades_at_edges() {
        let w = hann_window(8);
        assert!(w[0].abs() < 1e-4);
        for i in 0..4 {
            assert!((w[i] - w[7 - i]).abs() < 1e-6, "simetría");
        }
        // Forma cerrada con denominador N-1: w[3] = ½(1−cos(6π/7)).
        let expected = 0.5 * (1.0 - (6.0 * std::f64::consts::PI / 7.0).cos());
        assert!((w[3] as f64 - expected).abs() < 1e-6);
    }

    #[test]
    fn sine_peak_lands_in_expected_bin() {
        let mut analyzer = SpectrumAnalyzer::new(2048);
        // 440 Hz con ventana de 2048 @44.1k: resolución ≈ 21.53 Hz → bin 20.4.
        let frame = sine(440.0, 2048.0 / SR, 0.8);
        let mags = analyzer.magnitudes(&frame);
        assert_eq!(mags.len(), 1024);

        let (best_bin, &best_mag) = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!(
            (18..=23).contains(&best_bin),
            "pico en bin {best_bin}, esperado ~20"
        );
        // Dominancia: el pico supera por mucho la mediana del espectro.
        let mut sorted = mags.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[512];
        assert!(best_mag > median * 50.0, "el tono domina el espectro");
    }

    #[test]
    fn silence_yields_near_zero_spectrum() {
        let mut analyzer = SpectrumAnalyzer::new(1024);
        let mags = analyzer.magnitudes(&vec![0.0; 1024]);
        assert!(mags.iter().all(|m| *m < 1e-9));
    }

    #[test]
    fn two_tones_separate_low_and_high_energy() {
        let mut analyzer = SpectrumAnalyzer::new(4096);
        let sr = SR;
        let n = 4096;
        let frame: Vec<f32> = (0..n)
            .map(|i| {
                0.5 * (2.0 * std::f32::consts::PI * 100.0 * i as f32 / sr).sin()
                    + 0.5 * (2.0 * std::f32::consts::PI * 4000.0 * i as f32 / sr).sin()
            })
            .collect();
        let mags = analyzer.magnitudes(&frame);
        let bin_of = |f: f32| (f / (sr / n as f32)).round() as usize;

        let low = mags[bin_of(100.0)];
        let high = mags[bin_of(4000.0)];
        // Un punto medio sin contenido (p. ej. 1 kHz) queda muy por debajo.
        let mid = mags[bin_of(1000.0)];
        assert!(low > mid * 30.0 && high > mid * 30.0);
    }
}
