//! Afinidad del usuario: artista, género, acústico, match directo.

use std::collections::HashMap;

use crate::domain::track::Track;
use crate::infrastructure::storage::TrackListeningStats;
use crate::recommendation::types::{AcousticProfile, TrackAcousticProfile};

/// Afinidad entre un track y el historial del usuario.
///
/// ```text
/// affinity = 0.4 · artist_affinity
///          + 0.3 · genre_affinity
///          + 0.2 · acoustic_affinity
///          + 0.1 · direct_match
/// ```
pub fn user_affinity(
    track: &Track,
    history: &[TrackListeningStats],
    user_acoustic: &AcousticProfile,
    track_profiles: &HashMap<i64, TrackAcousticProfile>,
) -> f64 {
    let artist_affinity = artist_affinity_score(track, history);
    let genre_affinity = genre_affinity_score(track, history);
    let acoustic_affinity_val = acoustic_affinity_score(track, track_profiles, user_acoustic);
    let direct_match = direct_match_score(track, history);

    (0.4 * artist_affinity
        + 0.3 * genre_affinity
        + 0.2 * acoustic_affinity_val
        + 0.1 * direct_match) as f64
}

fn artist_affinity_score(track: &Track, history: &[TrackListeningStats]) -> f64 {
    if history.is_empty() {
        return 0.0;
    }
    let track_artist_names: Vec<&str> = track
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    if track_artist_names.is_empty() {
        return 0.0;
    }

    let mut artist_plays: HashMap<String, i64> = HashMap::new();
    let mut total_plays: i64 = 0;
    for entry in history {
        if let Some(ref name) = entry.artist_name {
            *artist_plays.entry(name.clone()).or_insert(0) += entry.play_count;
            total_plays += entry.play_count;
        }
    }
    if total_plays == 0 {
        return 0.0;
    }

    let matching_plays: i64 = track_artist_names
        .iter()
        .filter_map(|name| artist_plays.get(*name).copied())
        .sum();

    matching_plays as f64 / total_plays as f64
}

fn genre_affinity_score(track: &Track, history: &[TrackListeningStats]) -> f64 {
    if history.is_empty() {
        return 0.0;
    }
    let track_genre_names: Vec<&str> = track.genres.iter().map(|g| g.name.as_str()).collect();
    if track_genre_names.is_empty() {
        return 0.0;
    }

    // Contar plays por género usando el nombre del artista como proxy
    let mut genre_plays: HashMap<String, i64> = HashMap::new();
    let mut total_plays: i64 = 0;
    for entry in history {
        if let Some(ref name) = entry.artist_name {
            *genre_plays.entry(name.clone()).or_insert(0) += entry.play_count;
            total_plays += entry.play_count;
        }
    }
    if total_plays == 0 {
        return 0.0;
    }

    let matching_plays: i64 = track_genre_names
        .iter()
        .filter_map(|name| genre_plays.get(*name).copied())
        .sum();

    matching_plays as f64 / total_plays as f64
}

fn acoustic_affinity_score(
    track: &Track,
    track_profiles: &HashMap<i64, TrackAcousticProfile>,
    user_acoustic: &AcousticProfile,
) -> f64 {
    if user_acoustic.weight_sum < f32::EPSILON {
        return 0.0;
    }
    // Buscar el perfil acústico del track directamente o en el mapa
    let profile = if let Some(p) = track_profiles.get(&track.id) {
        p.clone()
    } else {
        return 0.5;
    };

    let track_vec = crate::recommendation::types::FeatureVector::from_profile(&profile);
    let user_vec = user_acoustic.to_vector();

    let mag_t = track_vec.magnitude();
    let mag_u = user_vec.magnitude();

    if mag_t < f32::EPSILON || mag_u < f32::EPSILON {
        return 0.0;
    }

    let dot = track_vec.dot(&user_vec);
    let cosine = dot / (mag_t * mag_u);
    cosine.clamp(0.0, 1.0) as f64
}

fn direct_match_score(track: &Track, history: &[TrackListeningStats]) -> f64 {
    if history.is_empty() {
        return 0.0;
    }
    let key = track.identifier();
    for entry in history {
        if entry.key == key {
            if entry.recently_played && entry.play_count >= 2 {
                return 1.0;
            }
            if entry.play_count >= 2 {
                return 0.8;
            }
            return 0.5;
        }
    }
    0.0
}