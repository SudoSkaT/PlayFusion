//! Soporte de tests para la capa analysis: generadores de señales sintéticas
//! (spec §38: seno, silencio, impulso, multifrecuencia, variación de amplitud).
//!
//! `#![cfg(test)]`-gated a nivel de módulo en `mod.rs`; las funciones son
//! puras y deterministas.

#![cfg(test)]

/// Seno puro con fase implícita por muestra (`t` explícito en segundos).
pub fn sine_wave(freq_hz: f32, sample_rate: f32, t_secs: f32, amplitude: f32) -> f32 {
    // `sample_rate` queda en la firma por simetría con los generadores que
    // muestrean por índice; la fase aquí es continua.
    let _ = sample_rate;
    amplitude * (2.0 * std::f32::consts::PI * freq_hz * t_secs).sin()
}

/// Buffer de `n` muestras de un seno.
pub fn sine(freq_hz: f32, sample_rate: f32, n: usize, amplitude: f32) -> Vec<f32> {
    (0..n)
        .map(|i| sine_wave(freq_hz, sample_rate, i as f32 / sample_rate, amplitude))
        .collect()
}

/// Silencio digital exacto.
pub fn silence(n: usize) -> Vec<f32> {
    vec![0.0; n]
}

/// Impulsos discretos cada `period` muestras (clicks de amplitud `amp`).
pub fn impulse_train(n: usize, period: usize, amp: f32) -> Vec<f32> {
    (0..n)
        .map(|i| if i % period == 0 { amp } else { 0.0 })
        .collect()
}

/// Mezcla ponderada de varias frecuencias.
pub fn mix(freqs: &[(f32 /*hz*/, f32 /*amp*/)], sample_rate: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate;
            freqs
                .iter()
                .map(|&(f, a)| sine_wave(f, sample_rate, t, a))
                .sum()
        })
        .collect()
}

/// Rampa lineal de amplitud de `from` a `to` en `n` muestras (portadora
/// sinusoidal opcional para que el RMS sea significativo).
pub fn ramp(freq_hz: f32, sample_rate: f32, n: usize, from: f32, to: f32) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let g = from + (to - from) * i as f32 / (n.max(2) - 1) as f32;
            sine_wave(freq_hz, sample_rate, i as f32 / sample_rate, g)
        })
        .collect()
}
