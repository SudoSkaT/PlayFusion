//! Bandas de frecuencia: energía relativa por banda (ratios 0..1).
//!
//! El valor de cada banda es `energía_banda / energía_total` del frame — un
//! ratio acotado y comparable entre pistas, que el ParameterMapper de
//! visualización curvará/escalará (spec §23). Los bordes son configurables.

/// Bordes de banda en Hz (5 bandas contiguas → 6 bordes).
#[derive(Debug, Clone, Copy)]
pub struct BandEdges {
    pub edges: [f32; 6],
}

impl Default for BandEdges {
    fn default() -> Self {
        // División habitual para features musicales.
        Self {
            edges: [20.0, 250.0, 500.0, 2000.0, 4000.0, 16_000.0],
        }
    }
}

/// Energía relativa por banda (todas 0..1; la suma puede ser <1 si hay
/// contenido fuera de [edges[0], edges[5]]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BandRatios {
    pub bass: f32,
    pub low_mid: f32,
    pub mid: f32,
    pub high_mid: f32,
    pub high: f32,
}

impl BandRatios {
    pub fn as_array(&self) -> [f32; 5] {
        [self.bass, self.low_mid, self.mid, self.high_mid, self.high]
    }
}

/// Calcula los ratios por banda a partir de magnitudes lineales.
///
/// `sample_rate` es la tasa de la SEÑAL analizada (post-decodificador) y
/// `fft_size` el tamaño de ventana usado para producir `magnitudes`.
pub fn band_ratios(magnitudes: &[f32], sample_rate: f32, edges: &BandEdges) -> BandRatios {
    let nyquist = sample_rate / 2.0;
    let bin_hz = nyquist / magnitudes.len() as f32;

    // Potencia por bin (m²) acumulada en la banda correspondiente.
    let mut powers = [0.0f32; 5];
    let mut total = 0.0f32;
    for (i, m) in magnitudes.iter().enumerate() {
        let p = m * m;
        total += p;
        let freq = i as f32 * bin_hz;
        if freq < edges.edges[0] || freq > edges.edges[5] {
            continue;
        }
        // Banda por búsqueda lineal sobre 6 bordes (barato, sin allocs).
        for (b, window_end) in edges.edges[1..].iter().enumerate() {
            if freq <= *window_end {
                powers[b] += p;
                break;
            }
        }
    }

    if total <= f32::EPSILON {
        return BandRatios::default();
    }
    let inv = 1.0 / total;
    BandRatios {
        bass: powers[0] * inv,
        low_mid: powers[1] * inv,
        mid: powers[2] * inv,
        high_mid: powers[3] * inv,
        high: powers[4] * inv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = 4096;
    const SR: f32 = 44_100.0;

    /// Espectro sintético: dos picos limpios en las frecuencias dadas.
    fn spectrum_with(peaks: &[(f32 /*hz*/, f32 /*amp*/)]) -> Vec<f32> {
        let mut mags = vec![0.0; N / 2];
        let bin_hz = (SR / 2.0) / mags.len() as f32;
        for &(freq, amp) in peaks {
            let bin = (freq / bin_hz).round() as usize;
            mags[bin] += amp;
        }
        mags
    }

    #[test]
    fn silence_gives_all_zero_ratios() {
        let mags = vec![0.0; N / 2];
        let r = band_ratios(&mags, SR, &BandEdges::default());
        assert_eq!(r, BandRatios::default());
    }

    #[test]
    fn pure_bass_concentrates_in_bass_band() {
        // 80 Hz: dentro de [20,250].
        let mags = spectrum_with(&[(80.0, 1.0)]);
        let r = band_ratios(&mags, SR, &BandEdges::default());
        assert!(r.bass > 0.95, "bass={}", r.bass);
        assert!(r.high < 0.01 && r.mid < 0.01);
    }

    #[test]
    fn two_tones_split_by_band() {
        let mags = spectrum_with(&[(100.0, 1.0), (5000.0, 3.0)]);
        let r = band_ratios(&mags, SR, &BandEdges::default());
        // La potencia escala con amp²: 5000 Hz domina 9:1.
        assert!(r.high > r.bass * 4.0, "high={} bass={}", r.high, r.bass);
        let sum: f32 = r.as_array().iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-4,
            "los dos picos están dentro de las bandas"
        );
    }

    #[test]
    fn content_outside_edges_is_counted_in_total_but_not_in_bands() {
        // Pico a 18 kHz (fuera del borde superior 16k) y otro a 100 Hz.
        let mags = spectrum_with(&[(100.0, 1.0), (18_000.0, 1.0)]);
        let r = band_ratios(&mags, SR, &BandEdges::default());
        let sum: f32 = r.as_array().iter().sum();
        assert!(
            (sum - 0.5).abs() < 0.02,
            "mitad de la potencia queda fuera de bandas: sum={sum}"
        );
    }
}
