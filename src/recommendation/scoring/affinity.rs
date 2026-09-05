//! Afinidad del usuario hacia un track concreto.
//!
//! Diseño (FASE 9): `metadata_similarity` ya mide la similitud de artista,
//! género, álbum y año vía el perfil. `user_affinity` NO repite eso; mide la
//! RELACIÓN DIRECTA del usuario con este track:
//!   - engagement directo (cuánto y qué tan reciente lo escuchó),
//!   - afinidad acústica (features del track respecto al perfil acústico).
//!
//! Así evitamos el bug previo que usaba el nombre del artista como proxie de
//! género (no hay datos de género por track en el historial) y separamos dos
//! señales diferentes: "parecido a lo que te gusta" (metadata) vs. "tú, con
//! este track" (affinity).

use std::collections::HashMap;

use crate::domain::track::Track;
use crate::infrastructure::storage::TrackListeningStats;
use crate::recommendation::recency_bonus;
use crate::recommendation::scoring::recency::days_since;
use crate::recommendation::types::{AcousticProfile, TrackAcousticProfile};

/// Afinidad entre un track y el usuario.
///
/// ```text
/// affinity = 0.6 · engagement   + 0.4 · acoustic_affinity
/// engagement = engagement_score(track, history)
/// ```
///
/// `engagement` pesa más porque refleja interacción real; la afinidad acústica
/// refuerza cuando hay perfil de features suficiente.
pub fn user_affinity(
    track: &Track,
    history: &[TrackListeningStats],
    user_acoustic: &AcousticProfile,
    track_profiles: &HashMap<i64, TrackAcousticProfile>,
) -> f64 {
    let engagement = engagement_score(track, history);
    let acoustic = acoustic_affinity_score(track, track_profiles, user_acoustic);
    0.6 * engagement + 0.4 * acoustic
}

/// Interacción directa del usuario con el track: frecuencia × recencia.
///
/// ```text
/// engagement = normalized_plays · recency
/// normalized_plays = log1p(play_count) / log1p(max_history_plays)
/// recency = exp(-λ · days_since_last_played)
/// ```
///
/// Un 1.0 solo si el track es (a la vez) de los más escuchados y reciente.
/// La normalización log evita que un single muy escuchado eclipsen al resto.
fn engagement_score(track: &Track, history: &[TrackListeningStats]) -> f64 {
    if history.is_empty() {
        return 0.0;
    }
    let key = track.identifier();
    let mut play_count = 0i64;
    let mut last_played: Option<&str> = None;
    let mut max_plays = 1i64;
    for entry in history {
        if entry.play_count > max_plays {
            max_plays = entry.play_count;
        }
        if entry.key == key {
            play_count += entry.play_count;
            if last_played.is_none() {
                last_played = Some(&entry.last_played);
            }
        }
    }
    if play_count <= 0 {
        return 0.0;
    }
    let normalized = (play_count as f64 + 1.0).ln_1p() / (max_plays as f64 + 1.0).ln_1p();
    let recency = last_played
        .map(|lp| recency_bonus(days_since(lp)))
        .unwrap_or(0.0);
    (normalized * recency).clamp(0.0, 1.0)
}

fn acoustic_affinity_score(
    track: &Track,
    track_profiles: &HashMap<i64, TrackAcousticProfile>,
    user_acoustic: &AcousticProfile,
) -> f64 {
    if user_acoustic.weight_sum < f32::EPSILON {
        return 0.0;
    }
    let profile = match track_profiles.get(&track.id) {
        Some(p) => p,
        // Sin perfil acústico del track, no podemos afirmar afinidad acústica.
        None => return 0.0,
    };

    let track_vec = crate::recommendation::types::FeatureVector::from_profile(profile);
    let user_vec = user_acoustic.to_vector();

    let mag_t = track_vec.magnitude();
    let mag_u = user_vec.magnitude();

    if mag_t < f32::EPSILON || mag_u < f32::EPSILON {
        return 0.0;
    }

    let cosine = track_vec.dot(&user_vec) / (mag_t * mag_u);
    cosine.clamp(0.0, 1.0) as f64
}
