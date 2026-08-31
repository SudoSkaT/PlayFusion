//! Sistema de recomendaciones local (FASE 8–10).
//!
//! Pipeline: candidatos → scoring (metadata + acoustic + affinity + recency + popularity - negative) → ranking.

pub mod scoring;
pub mod profile;
pub mod ranker;
pub mod types;

pub use profile::UserProfile;
pub use ranker::rank;
pub use types::{
    AcousticProfile, Candidate, FeatureVector, RecommendationScore, ScoreComponents,
    TrackAcousticProfile,
};
pub use scoring::{
    acoustic_similarity, metadata_similarity, negative_penalty, popularity_factor,
    recency_bonus, user_affinity,
};