-- 0003: PlayFusion pasa a usar únicamente YouTube/YouTube Music (rustypipe).
--
-- Se eliminan las columnas de proveedores descartados (Spotube/Piped, Jellyfin,
-- Koel, MusicBrainz) y se introduce `youtube_id` como único ID externo. Se
-- añade el soporte para playlists locales, override local de carátula y cache
-- de letras.
--
-- Nota sobre datos: `providers`, `tags.source` e `history.source` referencian
-- proveedores eliminados. Las filas de `history` se conservan (registro de
-- reproducción); los identificadores de proveedores obsoletos se descartan.

PRAGMA foreign_keys = ON;

-- Reconstruye `providers`: conserva los track_id existentes y coloca un único
-- id externo `youtube_id`.
CREATE TABLE providers_new (
    track_id   INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    youtube_id TEXT UNIQUE
);

INSERT INTO providers_new (track_id)
SELECT track_id FROM providers;

DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;

-- Playlists locales (no sincronizadas con YouTube).
CREATE TABLE IF NOT EXISTS playlists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id)    ON DELETE CASCADE,
    position    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (playlist_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist ON playlist_tracks(playlist_id);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_track    ON playlist_tracks(track_id);

-- Override local de la imagen de portada de una pista (sobre la de YouTube).
CREATE TABLE IF NOT EXISTS artwork_overrides (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    image    TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Cache local de letras (fetch una vez desde YouTube Music).
CREATE TABLE IF NOT EXISTS lyrics_cache (
    track_id  INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    body      TEXT NOT NULL,
    footer    TEXT,
    fetched_at TEXT NOT NULL DEFAULT (datetime('now'))
);