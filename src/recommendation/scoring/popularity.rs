//! Factor de popularidad: log-normalizado.

/// Popularidad normalizada por decil.
///
/// ```text
/// popularity = log1p(play_count) / log1p(max_play_count)
/// ```
///
/// Evita que los tracks con 10000 reproducciones dominen sobre los que
/// tienen 100, pero mantiene una señal de popularidad moderada.
pub fn popularity_factor(play_count: i64, max_play_count: i64) -> f64 {
    if max_play_count <= 0 {
        return 0.0;
    }
    let log_count = (play_count as f64 + 1.0).ln_1p();
    let log_max = (max_play_count as f64 + 1.0).ln_1p();
    (log_count / log_max).clamp(0.0, 1.0)
}