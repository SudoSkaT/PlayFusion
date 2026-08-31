-- 0007: perfiles acústicos por track para el sistema de recomendaciones (FASE 8).
--
-- Almacena el vector de features promediado por track, calculado una vez
-- a partir de los frames de AudioFeatures del análisis. Permite comparar
-- perfiles acústicos entre tracks sin recalcular por frame.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS track_acoustic_profiles (
    track_id              INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    rms_mean              REAL    NOT NULL DEFAULT 0.0,
    bass_mean             REAL    NOT NULL DEFAULT 0.0,
    low_mid_mean          REAL    NOT NULL DEFAULT 0.0,
    mid_mean              REAL    NOT NULL DEFAULT 0.0,
    high_mid_mean         REAL    NOT NULL DEFAULT 0.0,
    high_mean             REAL    NOT NULL DEFAULT 0.0,
    spectral_centroid_mean REAL NOT NULL DEFAULT 0.0,
    bpm_mean              REAL    NOT NULL DEFAULT 0.0,
    bpm_variance          REAL    NOT NULL DEFAULT 0.0,
    onset_mean            REAL    NOT NULL DEFAULT 0.0,
    band_profile          TEXT    NOT NULL DEFAULT '[0.0,0.0,0.0,0.0,0.0]',
    frame_count           INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_tacp_track_id ON track_acoustic_profiles(track_id);