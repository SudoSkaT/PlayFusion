//! Penalización por señales negativas (FASE 9).
//!
//! Se basa en señales REALES de interacción (skips contextualmente
//! significativos / unlikes) frente a intentos de reproducción, no en umbrales
//! arbitrarios de play-count. La tasa se define sobre intentos de escuchar
//! ("plays"), no sobre completions, para no confundir "no terminó" con "evitar".

/// Penalización multiplicativa para un track dado sus señales negativas.
///
/// ```text
/// let attempts = max(play_signals, 1)
/// let skip_rate = skip_signals / attempts            // proporción de disgusto
/// penalty = 1 / (1 + k · skip_rate)                  // k = 3.0
/// ```
///
/// La función `1/(1 + k·r)` baja suavemente de 1.0 (r=0, sin skips) hacia 0 a
/// medida que sube la tasa de skip, sin un corte arbitrario y sin llegar nunca
/// a cero. Con `r=0.25`: 0.57; `r=0.5`: 0.40; `r=1.0`: 0.25.
///
/// Los `skip_signals` deben ser SOLO skips contextualmente significativos
/// (`is_meaningful_negative`), no los skips de autoplay que no indican disgusto.
pub fn negative_penalty(skip_signals: i64, play_signals: i64) -> f64 {
    let attempts = play_signals.max(1) as f64;
    let skips = skip_signals.max(0) as f64;
    let rate = (skips / attempts).min(1.0);
    const K: f64 = 3.0;
    1.0 / (1.0 + K * rate)
}

/// Conteo de señales negativas significativas y de intentos (plays + rec_clicks).
pub fn count_negative_ratio(
    negative_signals: i64,
    play_signals: i64,
) -> f64 {
    negative_penalty(negative_signals, play_signals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_skips_is_neutral() {
        assert!((negative_penalty(0, 10) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn more_skips_penalize_more() {
        assert!(negative_penalty(5, 10) < negative_penalty(2, 10));
        assert!(negative_penalty(5, 10) < 1.0);
    }

    #[test]
    fn never_reaches_zero_or_negative() {
        assert!(negative_penalty(1000, 1) > 0.0);
        assert!(negative_penalty(0, 0) > 0.0);
    }

    #[test]
    fn ratio_matches_direct() {
        assert_eq!(
            count_negative_ratio(3, 6),
            negative_penalty(3, 6)
        );
    }
}
