-- Índice compuesto para resolver recencia y frecuencia por canción sin
-- escanear el historial completo al pintar las listas.
CREATE INDEX IF NOT EXISTS idx_history_track_played_at
    ON history(track_id, played_at DESC, id DESC);
