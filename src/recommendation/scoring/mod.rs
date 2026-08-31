//! Componentes de scoring del sistema de recomendaciones.

pub mod metadata;
pub mod acoustic;
pub mod affinity;
pub mod recency;
pub mod popularity;
pub mod negative;

pub use metadata::metadata_similarity;
pub use acoustic::{acoustic_similarity, acoustic_similarity_to_profile};
pub use affinity::user_affinity;
pub use recency::recency_bonus;
pub use popularity::popularity_factor;
pub use negative::negative_penalty;