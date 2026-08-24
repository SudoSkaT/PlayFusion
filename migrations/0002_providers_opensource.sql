-- 0002: sustitución de columnas de plataformas propietarias por fuentes
-- open source (Spotube/Piped, Jellyfin, Koel).
--
-- Se reconstruye la tabla `providers` conservando `track_id` y
-- `musicbrainz_id` (no se borran filas existentes). Los IDs antiguos de
-- Spotify/YouTube/SoundCloud se descartan, tal como se reemplazó la capa de
-- proveedores.

CREATE TABLE providers_new (
    track_id       INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    spotube_id     TEXT UNIQUE,
    jellyfin_id    TEXT UNIQUE,
    koel_id        TEXT UNIQUE,
    musicbrainz_id TEXT UNIQUE
);

INSERT INTO providers_new (track_id, musicbrainz_id)
SELECT track_id, musicbrainz_id FROM providers;

DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;
