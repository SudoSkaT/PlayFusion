-- 0006: caché de streams resueltos.
--
-- Persiste las URLs de audio de googlevideo por video_id para que reproducir
-- de nuevo una canción (replay, autoplay, siguiente sesión) NO re-resuelva el
-- stream: YouTube las mantiene válidas durante horas y un GET de verificación
-- barato confirma que siguen vivas antes de usarlas. Así el caso común gasta 0
-- peticiones de resolución (player/visitor/PO) y la app sobrevive aunque
-- YouTube esté bloqueando la adquisición de visitor data en ese momento.

CREATE TABLE IF NOT EXISTS stream_cache (
    video_id   TEXT PRIMARY KEY,
    url        TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);