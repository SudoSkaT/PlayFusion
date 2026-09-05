//! Bonus de recencia: decaimiento exponencial.

use std::time::{Duration, SystemTime};

/// Decaimiento exponencial basado en días desde la última escucha.
///
/// ```text
/// days_since = now - last_played
/// recency = e^(-λ · days_since)    // λ = ln(2) / 14  (media: 14 días)
/// ```
///
/// Si nunca se escuchó: `recency = 0.0`.
pub fn recency_bonus(days_since: f64) -> f64 {
    if days_since <= 0.0 {
        return 1.0;
    }
    let lambda = (2.0_f64).ln() / 14.0;
    (-lambda * days_since).exp()
}

/// Calcula los días desde la última escucha a partir de una fecha SQL.
pub fn days_since(last_played: &str) -> f64 {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as f64;

    let parsed = parse_sql_datetime(last_played);
    let then = parsed.unwrap_or(0.0);
    let seconds_ago = now - then;
    seconds_ago / 86400.0
}

fn parse_sql_datetime(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.len() < 10 {
        return None;
    }
    let year = s[0..4].parse::<f64>().ok()?;
    let month = s[5..7].parse::<f64>().ok()?;
    let day = s[8..10].parse::<f64>().ok()?;
    let hour = s
        .get(11..13)
        .and_then(|x| x.parse::<f64>().ok())
        .unwrap_or(0.0);
    let minute = s
        .get(14..16)
        .and_then(|x| x.parse::<f64>().ok())
        .unwrap_or(0.0);
    let second = s
        .get(17..19)
        .and_then(|x| x.parse::<f64>().ok())
        .unwrap_or(0.0);

    // Aproximación: días desde epoch (ignora timezone)
    let days = (year - 1970.0) * 365.25
        + (month - 1.0) * 30.44
        + (day - 1.0)
        + hour / 24.0
        + minute / 1440.0
        + second / 86400.0;
    Some(days)
}
