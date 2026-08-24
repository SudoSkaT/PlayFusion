-- 0001: esquema inicial de PlayFusion
-- Bases: tracks, artists, albums, genres, providers (mapeo multi-plataforma a un
-- track_id interno canónico), tags (todos los tags/géneros de cada plataforma) y history.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS artists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    country     TEXT,
    biography   TEXT,
    image       TEXT
);

CREATE TABLE IF NOT EXISTS albums (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    title        TEXT    NOT NULL,
    release_date TEXT,
    cover        TEXT,
    label        TEXT
);

CREATE TABLE IF NOT EXISTS genres (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS tracks (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    title    TEXT NOT NULL,
    -- duración en milisegundos
    duration INTEGER,
    isrc     TEXT,
    album_id INTEGER REFERENCES albums(id) ON DELETE SET NULL
);

-- M:N: una canción puede tener varios artistas
CREATE TABLE IF NOT EXISTS track_artists (
    track_id  INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (track_id, artist_id)
);

-- M:N: una canción puede pertenecer a varios géneros
CREATE TABLE IF NOT EXISTS track_genres (
    track_id  INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    genre_id  INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
    PRIMARY KEY (track_id, genre_id)
);

-- M:N: un artista puede tener varios géneros
CREATE TABLE IF NOT EXISTS artist_genres (
    artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    genre_id  INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
    PRIMARY KEY (artist_id, genre_id)
);

-- Múltiples plataformas apuntan al mismo track_id interno (identificador canónico).
-- NULL significa que la canción no existe en esa plataforma.
CREATE TABLE IF NOT EXISTS providers (
    track_id       INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    spotify_id     TEXT UNIQUE,
    youtube_id     TEXT UNIQUE,
    soundcloud_id  TEXT UNIQUE,
    musicbrainz_id TEXT UNIQUE
);

-- Todos los tags/géneros reportados por cada plataforma para una canción.
CREATE TABLE IF NOT EXISTS tags (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    name     TEXT    NOT NULL,
    source   TEXT    NOT NULL,
    UNIQUE (track_id, name, source)
);

-- Historial de reproducción.
CREATE TABLE IF NOT EXISTS history (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id  INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    played_at TEXT    NOT NULL DEFAULT (datetime('now')),
    source    TEXT    NOT NULL,
    duration  INTEGER
);

CREATE INDEX IF NOT EXISTS idx_tracks_album    ON tracks(album_id);
CREATE INDEX IF NOT EXISTS idx_track_artists_artist ON track_artists(artist_id);
CREATE INDEX IF NOT EXISTS idx_track_genres_genre    ON track_genres(genre_id);
CREATE INDEX IF NOT EXISTS idx_tags_track            ON tags(track_id);
CREATE INDEX IF NOT EXISTS idx_history_track          ON history(track_id);
CREATE INDEX IF NOT EXISTS idx_history_played_at      ON history(played_at);
