//! Similaridad de metadata: artista, género, álbum, tag, década.

use std::collections::HashSet;
use chrono::Datelike;

use crate::domain::track::Track;
use crate::recommendation::types::UserProfile;

/// Similaridad de metadata entre un track y el perfil del usuario.
///
/// ```text
/// meta_sim = 0.35 · artist_match + 0.25 · genre_match
///          + 0.20 · album_match + 0.10 · tag_match
///          + 0.10 · decade_match
/// ```
pub fn metadata_similarity(track: &Track, profile: &UserProfile) -> f64 {
    let artist_match = artist_match_score(track, profile);
    let genre_match = genre_match_score(track, profile);
    let album_match = album_match_score(track, profile);
    let tag_match = tag_match_score(track, profile);
    let decade_match = decade_match_score(track, profile);

    (0.35 * artist_match
        + 0.25 * genre_match
        + 0.20 * album_match
        + 0.10 * tag_match
        + 0.10 * decade_match) as f64
}

fn artist_match_score(track: &Track, profile: &UserProfile) -> f64 {
    if profile.favorite_artists.is_empty() {
        return 0.0;
    }
    let profile_artists: HashSet<&str> = profile.favorite_artists.iter().map(|s| s.as_str()).collect();
    let match_count = track
        .artists
        .iter()
        .filter(|a| profile_artists.contains(a.name.as_str()))
        .count();
    if track.artists.is_empty() {
        return 0.0;
    }
    match_count as f64 / track.artists.len() as f64
}

fn genre_match_score(track: &Track, profile: &UserProfile) -> f64 {
    if profile.favorite_genres.is_empty() {
        return 0.0;
    }
    let profile_genres: HashSet<&str> = profile.favorite_genres.iter().map(|s| s.as_str()).collect();
    let match_count = track
        .genres
        .iter()
        .filter(|g| profile_genres.contains(g.name.as_str()))
        .count();
    if track.genres.is_empty() {
        return 0.0;
    }
    match_count as f64 / track.genres.len() as f64
}

fn album_match_score(track: &Track, profile: &UserProfile) -> f64 {
    if profile.favorite_albums.is_empty() {
        return 0.0;
    }
    if let Some(album_id) = track.album.as_ref().map(|a| a.id) {
        if profile.favorite_albums.contains(&album_id) {
            return 1.0;
        }
    }
    0.0
}

fn tag_match_score(track: &Track, profile: &UserProfile) -> f64 {
    if profile.favorite_tags.is_empty() {
        return 0.0;
    }
    let profile_tags: HashSet<&str> = profile.favorite_tags.iter().map(|s| s.as_str()).collect();
    let match_count = track
        .genres
        .iter()
        .filter(|g| profile_tags.contains(g.name.as_str()))
        .count();
    if track.genres.is_empty() {
        return 0.0;
    }
    match_count as f64 / track.genres.len() as f64
}

fn decade_match_score(track: &Track, profile: &UserProfile) -> f64 {
    if profile.favorite_decades.is_empty() {
        return 0.0;
    }
    let album_decade = track
        .album
        .as_ref()
        .and_then(|a| a.release_date)
        .map(|d| ((d.year() / 10) * 10) as i64);
    if let Some(decade) = album_decade {
        if profile.favorite_decades.contains(&decade) {
            return 1.0;
        }
    }
    0.0
}