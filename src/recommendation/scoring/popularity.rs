//! Factor de popularidad: log-normalizado.

/// Popularidad normalizada por decil.
///
/// ```text
/// popularity = ln1p(play_count) / ln1p(max_play_count)
/// ```
///
/// `ln1p` normaliza logs de forma que count=0 ⇒ 0 (sin popularidad) y la
/// relación es sublineal: un track con 10000 plays no domina sobre uno con
/// 100. Evita usar el play-count bruto, que desequilibraría el ranking.
pub fn popularity_factor(play_count: i64, max_play_count: i64) -> f64 {
    if max_play_count <= 0 {
        return 0.0;
    }
    let log_count = (play_count as f64).ln_1p();
    let log_max = (max_play_count as f64).ln_1p();
    (log_count / log_max).clamp(0.0, 1.0)
}