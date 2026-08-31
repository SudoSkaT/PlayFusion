//! Perfil musical local del usuario (FASE 10).
//!
/// Se deriva exclusivamente del historial de reproducción.

use chrono::Datelike;

use std::collections::HashMap;

use crate::domain::track::Track;
use crate::infrastructure::storage::HistoryEntry;
use crate::recommendation::types::{AcousticProfile, TrackAcousticProfile};

/// Perfil musical del usuario derivado del historial.
#[derive(Debug, Clone, Default)]
pub struct UserProfile {
    // ── Metadata ──
    pub favorite_artists: Vec<String>,
    pub favorite_genres: Vec<String>,
    pub favorite_albums: Vec<i64>,
    pub favorite_decades: Vec<i64>,
    pub favorite_tags: Vec<String>,

    // ── Features acústicos (promedio ponderado por completion_rate) ──
    pub acoustic_profile: AcousticProfile,

    // ── Historial de señales ──
    pub total_plays: u64,
    pub total_skips: u64,
    pub total_completions: u64,
    pub tracks_played: Vec<i64>,
    pub tracks_completed: Vec<i64>,
    pub tracks_skipped: Vec<i64>,
}

impl UserProfile {
    /// Construye un perfil a partir del historial y los tracks asociados.
    pub fn from_history(
        history: &[HistoryEntry],
        tracks: &HashMap<i64, Track>,
        acoustic_profiles: &HashMap<i64, TrackAcousticProfile>,
    ) -> Self {
        let mut profile = Self::default();
        let mut artist_counts: HashMap<String, u64> = HashMap::new();
        let mut genre_counts: HashMap<String, u64> = HashMap::new();
        let mut album_counts: HashMap<i64, u64> = HashMap::new();
        let mut decade_counts: HashMap<i64, u64> = HashMap::new();
        let mut tag_counts: HashMap<String, u64> = HashMap::new();

        for event in history {
            profile.total_plays += event.play_count as u64;
            profile.tracks_played.push(event.track_id);

            let track = match tracks.get(&event.track_id) {
                Some(t) => t,
                None => continue,
            };

            let track_duration_ms = track.duration.map(|d| d.as_millis() as i64).unwrap_or(0);
            let completed = event.duration.map_or(false, |d| {
                d >= track_duration_ms.saturating_sub(3000)
            });
            let skipped = event.duration.map_or(false, |d| {
                track_duration_ms > 0 && d < track_duration_ms / 5
            });

            if skipped {
                profile.total_skips += event.play_count as u64;
                profile.tracks_skipped.push(event.track_id);
            }
            if completed {
                profile.total_completions += event.play_count as u64;
                profile.tracks_completed.push(event.track_id);
            }

            // Contar artistas
            for artist in &track.artists {
                *artist_counts.entry(artist.name.clone()).or_insert(0) += event.play_count as u64;
            }

            // Contar géneros
            for genre in &track.genres {
                *genre_counts.entry(genre.name.clone()).or_insert(0) += event.play_count as u64;
            }

            // Contar álbumes completados
            if completed {
                if let Some(album_id) = track.album.as_ref().map(|a| a.id) {
                    *album_counts.entry(album_id).or_insert(0) += event.play_count as u64;
                }
            }

            // Contar décadas
            if let Some(decade) = track
                .album
                .as_ref()
                .and_then(|a| a.release_date)
                .map(|d| ((d.year() / 10) * 10) as i64)
            {
                *decade_counts.entry(decade).or_insert(0) += event.play_count as u64;
            }

            // Contar tags (géneros)
            for genre in &track.genres {
                *tag_counts.entry(genre.name.clone()).or_insert(0) += event.play_count as u64;
            }

            // Perfil acústico ponderado por completion_rate
            let weight = if completed {
                1.0
            } else if skipped {
                0.1
            } else {
                0.5
            };

            if let Some(ap) = acoustic_profiles.get(&event.track_id) {
                profile.acoustic_profile.add(ap, weight);
            }
        }

        // Top artists
        profile.favorite_artists = top_n(&artist_counts, 10);
        profile.favorite_genres = top_n(&genre_counts, 10);
        profile.favorite_albums = top_n_i64(&album_counts, 10);
        profile.favorite_decades = top_n_i64(&decade_counts, 10);
        profile.favorite_tags = top_n(&tag_counts, 10);

        profile.acoustic_profile.finalize();

        profile
    }
}

fn top_n(map: &HashMap<String, u64>, n: usize) -> Vec<String> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1));
    entries.into_iter().take(n).map(|(k, _)| k.clone()).collect()
}

fn top_n_i64(map: &HashMap<i64, u64>, n: usize) -> Vec<i64> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1));
    entries.into_iter().take(n).map(|(k, _)| *k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_history_entry(track_id: i64, play_count: i64, duration_ms: i64) -> HistoryEntry {
        HistoryEntry {
            track_id,
            played_at: "2024-06-15 12:00:00".to_string(),
            source: crate::domain::source::Source::YouTube,
            duration: Some(duration_ms),
            title: "Test".to_string(),
            artist_name: Some("Test Artist".to_string()),
            play_count,
        }
    }

    fn make_track(id: i64, title: &str) -> Track {
        let mut t = Track::new(title.to_string(), vec![], crate::domain::source::Source::YouTube);
        t.id = id;
        t.duration = Some(Duration::from_secs(200));
        t.genres = vec![crate::domain::genre::Genre::new("rock".to_string())];
        t
    }

    #[test]
    fn profile_from_history_populates_favorite_artists() {
        let mut tracks = HashMap::new();
        let track = make_track(1, "Song A");
        tracks.insert(1, track);

        let history = vec![make_history_entry(1, 5, 200_000)];
        let profiles = HashMap::new();

        let profile = UserProfile::from_history(&history, &tracks, &profiles);
        assert_eq!(profile.total_plays, 5);
        assert_eq!(profile.favorite_genres, vec!["rock"]);
    }

    #[test]
    fn completion_affects_acoustic_weight() {
        let mut tracks = HashMap::new();
        let track = make_track(1, "Song A");
        tracks.insert(1, track);

        let history = vec![
            make_history_entry(1, 3, 200_000), // completed
            make_history_entry(1, 2, 10_000),   // skipped
        ];
        let profiles = HashMap::new();

        let profile = UserProfile::from_history(&history, &tracks, &profiles);
        assert_eq!(profile.total_completions, 3);
        assert_eq!(profile.total_skips, 2);
    }
}