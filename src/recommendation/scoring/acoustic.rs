//! Similaridad acústica: distancia coseno entre perfiles de features.

use crate::recommendation::types::{FeatureVector, TrackAcousticProfile};

/// Similaridad acústica entre dos tracks vía distancia coseno.
///
/// ```text
/// acoustic_sim = 1 - cosine_distance(vec_a, vec_b)
/// ```
pub fn acoustic_similarity(a: &TrackAcousticProfile, b: &TrackAcousticProfile) -> f64 {
    let va = FeatureVector::from_profile(a);
    let vb = FeatureVector::from_profile(b);

    let mag_a = va.magnitude();
    let mag_b = vb.magnitude();

    if mag_a < f32::EPSILON || mag_b < f32::EPSILON {
        return 0.0;
    }

    let dot = va.dot(&vb);
    let cosine = dot / (mag_a * mag_b);
    cosine.clamp(0.0, 1.0) as f64
}

/// Similaridad entre el perfil acústico de un track y el perfil del usuario.
pub fn acoustic_similarity_to_profile(
    track_profile: &TrackAcousticProfile,
    user_profile: &crate::recommendation::types::AcousticProfile,
) -> f64 {
    if user_profile.weight_sum < f32::EPSILON {
        return 0.0;
    }
    let track_vec = FeatureVector::from_profile(track_profile);
    let user_vec = user_profile.to_vector();

    let mag_t = track_vec.magnitude();
    let mag_u = user_vec.magnitude();

    if mag_t < f32::EPSILON || mag_u < f32::EPSILON {
        return 0.0;
    }

    let dot = track_vec.dot(&user_vec);
    let cosine = dot / (mag_t * mag_u);
    cosine.clamp(0.0, 1.0) as f64
}
