//! Tipos centrales del sistema de recomendaciones (FASE 9–10).
//!
//! `FeatureVector` — representación numérica de un track para comparación acústica.
//! `UserProfile` — resumen del gusto del usuario derivado del historial.
//! `Candidate` — track candidato con su perfil acústico para scoring.
//! `RecommendationScore` — puntuación final con componentes desglosados.
//! `ScoreComponents` — desglose de cada componente del scoring.

use crate::domain::source::Source;
use crate::domain::track::Track;

/// Vector de features acústicas de un track (promedio de frames, normalizado).
///
/// Se usa para calcular la distancia coseno entre tracks en `acoustic_similarity`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeatureVector {
    pub rms: f32,
    pub bass: f32,
    pub low_mid: f32,
    pub mid: f32,
    pub high_mid: f32,
    pub high: f32,
    pub spectral_centroid: f32,
    pub bpm_norm: f32,
    pub onset: f32,
}

impl FeatureVector {
    /// Convierte un `TrackAcousticProfile` en un `FeatureVector` normalizado.
    pub fn from_profile(p: &TrackAcousticProfile) -> Self {
        Self {
            rms: p.rms_mean,
            bass: p.bass_mean,
            low_mid: p.low_mid_mean,
            mid: p.mid_mean,
            high_mid: p.high_mid_mean,
            high: p.high_mean,
            spectral_centroid: p.spectral_centroid_mean,
            bpm_norm: (p.bpm_mean / 200.0).clamp(0.0, 1.0),
            onset: p.onset_mean,
        }
    }

    /// Producto punto con otro vector (asumiendo normalizados).
    pub fn dot(&self, other: &Self) -> f32 {
        self.rms * other.rms
            + self.bass * other.bass
            + self.low_mid * other.low_mid
            + self.mid * other.mid
            + self.high_mid * other.high_mid
            + self.high * other.high
            + self.spectral_centroid * other.spectral_centroid
            + self.bpm_norm * other.bpm_norm
            + self.onset * other.onset
    }

    /// Magnitud del vector (asumiendo valores 0..1).
    pub fn magnitude(&self) -> f32 {
        (self.rms * self.rms
            + self.bass * self.bass
            + self.low_mid * self.low_mid
            + self.mid * self.mid
            + self.high_mid * self.high_mid
            + self.high * self.high
            + self.spectral_centroid * self.spectral_centroid
            + self.bpm_norm * self.bpm_norm
            + self.onset * self.onset)
            .sqrt()
    }
}

/// Perfil acústico agregado de un track (promedio ponderado de frames).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackAcousticProfile {
    pub track_id: i64,
    pub rms_mean: f32,
    pub bass_mean: f32,
    pub low_mid_mean: f32,
    pub mid_mean: f32,
    pub high_mid_mean: f32,
    pub high_mean: f32,
    pub spectral_centroid_mean: f32,
    pub bpm_mean: f32,
    pub bpm_variance: f32,
    pub onset_mean: f32,
    pub band_profile: [f32; 5],
    pub frame_count: i64,
}

/// Candidato a recomendación con su perfil acústico para scoring.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub track: Track,
    pub acoustic_profile: Option<TrackAcousticProfile>,
}

impl Candidate {
    pub fn new(track: Track) -> Self {
        Self {
            acoustic_profile: None,
            track,
        }
    }

    pub fn with_acoustic(mut self, profile: TrackAcousticProfile) -> Self {
        self.acoustic_profile = Some(profile);
        self
    }
}

/// Score final de una recomendación con desglose de componentes.
#[derive(Debug, Clone)]
pub struct RecommendationScore {
    pub track_id: i64,
    pub final_score: f64,
    pub components: ScoreComponents,
}

/// Componentes individuales del scoring.
#[derive(Debug, Clone, Default)]
pub struct ScoreComponents {
    pub metadata: f64,
    pub acoustic: f64,
    pub affinity: f64,
    pub recency: f64,
    pub popularity: f64,
    pub negative: f64,
}

impl ScoreComponents {
    pub fn weighted_sum(
        &self,
        w_meta: f64,
        w_acoustic: f64,
        w_affinity: f64,
        w_recency: f64,
        w_popularity: f64,
    ) -> f64 {
        w_meta * self.metadata
            + w_acoustic * self.acoustic
            + w_affinity * self.affinity
            + w_recency * self.recency
            + w_popularity * self.popularity
    }
}

/// Perfil musical del usuario derivado del historial de reproducción.
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

#[derive(Debug, Clone, Default)]
pub struct AcousticProfile {
    pub rms: f32,
    pub bass: f32,
    pub low_mid: f32,
    pub mid: f32,
    pub high_mid: f32,
    pub high: f32,
    pub spectral_centroid: f32,
    pub bpm_mean: f32,
    pub bpm_variance: f32,
    pub onset_mean: f32,
    pub band_profile: [f32; 5],
    pub weight_sum: f32,
}

impl AcousticProfile {
    pub fn add(&mut self, features: &TrackAcousticProfile, weight: f32) {
        self.rms += weight * features.rms_mean;
        self.bass += weight * features.bass_mean;
        self.low_mid += weight * features.low_mid_mean;
        self.mid += weight * features.mid_mean;
        self.high_mid += weight * features.high_mid_mean;
        self.high += weight * features.high_mean;
        self.spectral_centroid += weight * features.spectral_centroid_mean;
        self.bpm_mean += weight * features.bpm_mean;
        self.bpm_variance += weight * features.bpm_variance;
        self.onset_mean += weight * features.onset_mean;
        for i in 0..5 {
            self.band_profile[i] += weight * features.band_profile[i];
        }
        self.weight_sum += weight;
    }

    pub fn finalize(&mut self) {
        let w = self.weight_sum.max(1.0);
        self.rms /= w;
        self.bass /= w;
        self.low_mid /= w;
        self.mid /= w;
        self.high_mid /= w;
        self.high /= w;
        self.spectral_centroid /= w;
        self.bpm_mean /= w;
        self.bpm_variance /= w;
        self.onset_mean /= w;
        for i in 0..5 {
            self.band_profile[i] /= w;
        }
    }

    pub fn to_vector(&self) -> FeatureVector {
        FeatureVector {
            rms: self.rms,
            bass: self.bass,
            low_mid: self.low_mid,
            mid: self.mid,
            high_mid: self.high_mid,
            high: self.high,
            spectral_centroid: self.spectral_centroid,
            bpm_norm: (self.bpm_mean / 200.0).clamp(0.0, 1.0),
            onset: self.onset_mean,
        }
    }
}

/// Evento de reproducción con contexto de duración del track para calcular completion/skip.
#[derive(Debug, Clone)]
pub struct HistoryPlayEvent {
    pub track_id: i64,
    pub played_at: String,
    pub source: Source,
    pub duration: Option<i64>,
    pub track_duration: Option<i64>,
    pub artist_name: Option<String>,
}