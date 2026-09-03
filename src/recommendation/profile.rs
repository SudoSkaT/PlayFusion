//! Perfil musical local del usuario (FASE 10).
//!
//! Se deriva EXCLUSIVAMENTE de los eventos de interacción (`PlaySignal`), no
//! de una sesión manual. Cada señal pesa según su tipo y contexto: `play` no
//! equivale a `like`, y `skip` en autoplay no equivale a disgusto.
//!
//! Reemplaza la construcción a partir de `history` (que mezclaba play/complete/
//! skip en una sola fila por reproducción con `duration` = duración total, lo
//! cual no permitía distinguirlos).

use chrono::Datelike;

use std::collections::HashMap;

use crate::domain::track::Track;
use crate::recommendation::signals::{signal_weight, is_meaningful_negative, PlaySignal, SignalKind};
use crate::recommendation::types::TrackAcousticProfile;

/// Cómo agregar la preferencia por clave (artista/género/etc.).
fn accumulate(
    counts: &mut HashMap<String, f32>,
    keys: &[String],
    weight: f32,
) {
    for k in keys {
        *counts.entry(k.clone()).or_insert(0.0) += weight;
    }
}

fn top_n(counts: &HashMap<String, f32>, n: usize) -> Vec<String> {
    let mut entries: Vec<_> = counts.iter().collect();
    entries.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    entries.into_iter().take(n).map(|(k, _)| k.clone()).collect()
}

fn add_track_counts(profile: &mut crate::recommendation::types::UserProfile, track: &Track, weight: f32) {
    let artist_keys: Vec<String> = track.artists.iter().map(|a| a.name.clone()).collect();
    let genre_keys: Vec<String> = track.genres.iter().map(|g| g.name.clone()).collect();
    accumulate(&mut profile._artist_w, &artist_keys, weight);
    accumulate(&mut profile._genre_w, &genre_keys, weight);
    if let Some(album_id) = track.album.as_ref().map(|a| a.id) {
        *profile._album_w.entry(album_id).or_insert(0.0) += weight;
    }
    if let Some(decade) = track
        .album
        .as_ref()
        .and_then(|a| a.release_date)
        .map(|d| ((d.year() / 10) * 10) as i64)
    {
        *profile._decade_w.entry(decade).or_insert(0.0) += weight;
    }
    accumulate(&mut profile._tag_w, &genre_keys, weight);
}

impl crate::recommendation::types::UserProfile {
    /// Construye el perfil a partir de las señales de interacción y los tracks
    /// asociados. Requiere los perfiles acústicos para ponderar la preferencia
    /// acústica por la calidad de la señal.
    pub fn from_signals(
        signals: &[PlaySignal],
        tracks: &HashMap<i64, Track>,
        acoustic_profiles: &HashMap<i64, TrackAcousticProfile>,
    ) -> Self {
        let mut profile = Self::default();

        for signal in signals {
            let track = match tracks.get(&signal.track_id) {
                Some(t) => t,
                None => continue,
            };
            let completion = signal.is_completion();

            if let SignalKind::Replay = signal.signal {
                profile.total_replays += 1;
                profile.tracks_replayed.insert(signal.track_id);
            }
            if signal.signal == SignalKind::Completed {
                profile.total_completions += 1;
                profile.tracks_completed.insert(signal.track_id);
            }
            if signal.signal == SignalKind::Like {
                profile.total_likes += 1;
            }
            if signal.signal == SignalKind::Play || signal.signal == SignalKind::RecClick {
                profile.total_plays += 1;
                profile.tracks_played.insert(signal.track_id);
            }
            // Solo los skips contextualmente significativos cuentan como señal
            // negativa (no todo skip = disgusto).
            if is_meaningful_negative(signal.signal, signal.context) {
                profile.total_skips += 1;
                profile.tracks_skipped.insert(signal.track_id);
            }

            // El peso combina tipo + contexto + completado. Un skip en autoplay
            // no arrastra el peso acústico del track.
            let weight = signal_weight(signal.signal, signal.context, completion);
            if weight <= 0.0 {
                continue;
            }
            profile.total_weight += weight;
            add_track_counts(&mut profile, track, weight);

            if let Some(ap) = acoustic_profiles.get(&signal.track_id) {
                profile.acoustic_profile.add(ap, weight);
            }
        }

        profile.favorite_artists = top_n(&profile._artist_w, 12);
        profile.favorite_genres = top_n(&profile._genre_w, 12);
        profile.favorite_albums = {
            let mut entries: Vec<_> = profile._album_w.iter().collect();
            entries.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
            entries.into_iter().take(12).map(|(k, _)| *k).collect()
        };
        profile.favorite_decades = {
            let mut entries: Vec<_> = profile._decade_w.iter().collect();
            entries.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
            entries.into_iter().take(12).map(|(k, _)| *k).collect()
        };
        profile.favorite_tags = top_n(&profile._tag_w, 12);

        profile.acoustic_profile.finalize();

        profile
    }
}
