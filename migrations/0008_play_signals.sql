-- 0008: señales de interacción del usuario (FASE 11).
--
-- Modelo de métricas local que distingue señales reales de interacción que el
-- historial antiguo ($history) mezclaba: cada fila es UN evento de interacción
-- con UN track, con su contexto de reproducción. Diseñado para poder migrar a
-- un sistema remoto sin tocar el dominio: cada fila es autónoma y temporal.

PRAGMA foreign_keys = ON;

-- signal: tipo de interacción distinto (plays, completed, skip, replay, like,
--         unlike, playlist_add, rec_click, rec_impression)
-- context: circunstancias en que ocurrió (manual, queue, autoplay,
--          recommendation, search) — ver `PlayContext`.
-- recomm_id: opcional, cuando la señal proviene de una recomendación, une la
--            señal con la recomendación concreta (para evaluar click/impression).
-- duration_ms: ms realmente escuchados en ese evento (para completed/skip real).
CREATE TABLE IF NOT EXISTS play_signals (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    signal      TEXT    NOT NULL,
    context     TEXT    NOT NULL,
    at          TEXT    NOT NULL DEFAULT (datetime('now')),
    duration_ms INTEGER,
    recomm_id   INTEGER,
    track_duration_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_play_signals_track ON play_signals(track_id);
CREATE INDEX IF NOT EXISTS idx_play_signals_signal ON play_signals(signal);
CREATE INDEX IF NOT EXISTS idx_play_signals_at ON play_signals(at);
