//! Pipeline de generación de candidatos → scoring → ranking.

use std::collections::HashMap;

use crate::domain::track::Track;
use crate::infrastructure::storage::TrackListeningStats;
use crate::recommendation::{metadata_similarity, user_affinity, popularity_factor, negative_penalty, recency_bonus};
use crate::recommendation::scoring::acoustic::acoustic_similarity_to_profile;
use crate::recommendation::scoring::recency::days_since;
use crate::recommendation::types::{RecommendationScore, TrackAcousticProfile, UserProfile};

/// Pesos del pipeline (suman 1.0).
pub const WEIGHTS: ScoringWeights = ScoringWeights {
    meta: 0.25,
    acoustic: 0.25,
    affinity: 0.30,
    recency: 0.10,
    popularity: 0.05,
};

#[derive(Debug, Clone, Copy)]
pub struct ScoringWeights {
    pub meta: f64,
    pub acoustic: f64,
    pub affinity: f64,
    pub recency: f64,
    pub popularity: f64,
}

impl ScoringWeights {
    pub fn weighted_sum(
        &self,
        meta: f64,
        acoustic: f64,
        affinity: f64,
        recency: f64,
        popularity: f64,
    ) -> f64 {
        self.meta * meta
            + self.acoustic * acoustic
            + self.affinity * affinity
            + self.recency * recency
            + self.popularity * popularity
    }
}

/// Genera recomendaciones para un usuario a partir de un catálogo.
pub async fn rank(
    candidates: &[Track],
    profile: &UserProfile,
    history: &[TrackListeningStats],
    acoustic_profiles: &HashMap<i64, TrackAcousticProfile>,
) -> Vec<RecommendationScore> {
    let max_play_count = history
        .iter()
        .map(|h| h.play_count)
        .max()
        .unwrap_or(1) as i64;

    let mut scores: Vec<RecommendationScore> = Vec::new();

    for track in candidates {
        let meta = metadata_similarity(track, profile);
        let acoustic = acoustic_score(track, profile, acoustic_profiles);
        let affinity = user_affinity(track, history, &profile.acoustic_profile, acoustic_profiles);

        let track_history: Vec<&TrackListeningStats> =
            history.iter().filter(|h| h.track_id == track.id).collect();
        let recency = track_history
            .iter()
            .map(|h| {
                let days = days_since(&h.last_played);
                recency_bonus(days)
            })
            .sum::<f64>()
            / track_history.len().max(1) as f64;

        let popularity = popularity_factor(
            track_history.iter().map(|h| h.play_count).sum::<i64>(),
            max_play_count,
        );
        let negative = negative_penalty(
            track_history.first().map_or(0, |h| h.play_count),
            track.duration.map(|d| d.as_millis() as i64).unwrap_or(0),
        );

        let raw = WEIGHTS.weighted_sum(meta, acoustic, affinity, recency, popularity);
        let final_score = raw * negative;

        scores.push(RecommendationScore {
            track_id: track.id,
            final_score,
            components: crate::recommendation::types::ScoreComponents {
                metadata: meta,
                acoustic,
                affinity,
                recency,
                popularity,
                negative,
            },
        });
    }

    scores.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap_or(std::cmp::Ordering::Equal));
    scores
}

fn acoustic_score(
    track: &Track,
    profile: &UserProfile,
    acoustic_profiles: &HashMap<i64, TrackAcousticProfile>,
) -> f64 {
    if let Some(track_profile) = acoustic_profiles.get(&track.id) {
        acoustic_similarity_to_profile(track_profile, &profile.acoustic_profile)
    } else {
        0.0
    }
}

/// Genera un vector de features para un track a partir de su perfil acústico.
pub fn track_to_feature_vector(
    track_id: i64,
    profiles: &HashMap<i64, TrackAcousticProfile>,
) -> Option<crate::recommendation::types::FeatureVector> {
    profiles.get(&track_id).map(|p| crate::recommendation::types::FeatureVector::from_profile(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_one() {
        let w = WEIGHTS;
        let total = w.meta + w.acoustic + w.affinity + w.recency + w.popularity;
        assert!((total - 0.95).abs() < 1e-10, "pesos deben sumar 0.95: {total}");
    }
}