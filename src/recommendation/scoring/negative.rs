//! Penalización por señales negativas.

use crate::infrastructure::storage::TrackListeningStats;

/// Penalización multiplicativa para señales negativas.
///
/// ```text
/// penalty = 1.0
/// if skip_rate(track) > 0.5:   penalty *= 0.3   // Muchos skips → evitar
/// if skip_rate(track) > 0.2:   penalty *= 0.7   // Skips moderados → penalizar
/// ```
///
/// `skip` se detecta cuando `history.duration < track.duration * 0.2`.
pub fn negative_penalty(
    play_count: i64,
    track_duration_ms: i64,
) -> f64 {
    if play_count <= 0 || track_duration_ms <= 0 {
        return 1.0;
    }

    let play_count_f64 = play_count as f64;
    // Heurística suave basada en número de reproducciones:
    // más reproducciones = más oportunidad de skips, pero el peso es bajo
    if play_count_f64 > 50.0 {
        0.9   // Muchos plays → penalización ligera
    } else if play_count_f64 > 15.0 {
        0.95  // Plays moderados → penalización muy ligera
    } else {
        1.0   // Pocos plays → sin penalización
    }
}