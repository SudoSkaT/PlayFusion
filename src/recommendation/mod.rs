//! Sistema de recomendaciones local (FASE 8–10).
//!
//! Pipeline: candidatos → scoring (metadata + acoustic + affinity + recency + popularity - negative) → ranking.

pub mod acoustic_aggregator;
pub mod scoring;
pub mod profile;
pub mod ranker;
pub mod signals;
pub mod types;

#[cfg(test)]
pub mod behavior_tests;

pub use ranker::rank;
pub use signals::{
    signal_weight, aggregate_signals, is_meaningful_negative, PlayContext, PlaySignal, SignalKind,
};
pub use types::{
    AcousticProfile, Candidate, FeatureVector, RecommendationScore, ScoreComponents,
    TrackAcousticProfile, TrackSignals, UserProfile,
};
pub use scoring::{
    acoustic_similarity, metadata_similarity, negative_penalty, popularity_factor,
    recency_bonus, user_affinity,
};