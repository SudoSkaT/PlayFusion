//! Persistencia de entidades y eventos en SQLite.
//!
//! Funciones de escritura/lectura sobre el pool de [`Db`], usadas por la capa
//! de Aplicación. No contienen lógica de negocio, solo SQL.

use std::collections::HashMap;

use anyhow::Result;
use sqlx::sqlite::{Sqlite, SqliteQueryResult};
use sqlx::{Row, Transaction};

use crate::recommendation::types::TrackAcousticProfile;

use crate::domain::{album::Album, artist::Artist, source::Source, track::Track};

use super::db::Db;

/// Una entrada del historial de reproducción, con datos de display unidos.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub track_id: i64,
    pub played_at: String,
    pub source: Source,
    pub duration: Option<i64>,
    pub title: String,
    pub artist_name: Option<String>,
    /// Número total de reproducciones persistidas de esta canción.
    pub play_count: i64,
}

/// Resumen persistente de escucha para decorar cualquier copia de un track.
#[derive(Debug, Clone)]
pub struct TrackListeningStats {
    /// La misma clave estable usada por `Track::identifier()`.
    pub track_id: i64,
    pub key: String,
    pub artist_name: Option<String>,
    pub play_count: i64,
    pub last_played: String,
    pub recently_played: bool,
}

/// Una playlist local (no sincronizada con YouTube).
#[derive(Debug, Clone)]
pub struct PlaylistRow {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub track_count: i64,
}

impl Db {
    /// Inserta (o actualiza) una canción y sus relaciones de forma atómica.
    /// Devuelve el `id` interno canónico del track.
    pub async fn upsert_track(
        &self,
        track: &Track,
        provider_ids: &HashMap<Source, String>,
    ) -> Result<i64> {
        let mut tx = self.pool().begin().await?;
        let id = upsert_track_inner(&mut tx, track, provider_ids).await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Registra una reproducción en el historial.
    pub async fn record_history(
        &self,
        track_id: i64,
        source: Source,
        duration: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO history (track_id, played_at, source, duration) \
             VALUES (?1, datetime('now'), ?2, ?3)",
        )
        .bind(track_id)
        .bind(source.as_str())
        .bind(duration)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Devuelve las últimas `limit` reproducciones, ordenadas por fecha.
    pub async fn recent_history(&self, limit: i64) -> Result<Vec<HistoryEntry>> {
        let rows = sqlx::query(
            "SELECT h.track_id, h.played_at, h.source, h.duration, t.title, \
                    a.name AS artist_name, \
                    COUNT(h.id) OVER (PARTITION BY h.track_id) AS play_count \
             FROM history h \
             JOIN tracks t ON t.id = h.track_id \
             LEFT JOIN track_artists ta ON ta.track_id = t.id AND ta.position = 0 \
             LEFT JOIN artists a ON a.id = ta.artist_id \
             ORDER BY h.played_at DESC, h.id DESC \
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .iter()
            .map(|r| HistoryEntry {
                track_id: r.get("track_id"),
                played_at: r.get("played_at"),
                source: r.get::<Option<String>, _>("source")
                    .as_deref()
                    .map_or(Source::YouTube, |s| {
                        match s {
                            "youtube" => Source::YouTube,
                            _ => Source::YouTube,
                        }
                    }),
                duration: r.get("duration"),
                title: r.get("title"),
                artist_name: r.get("artist_name"),
                play_count: r.get("play_count"),
            })
            .collect())
    }

    /// Obtiene el perfil acústico de un track específico.
    pub async fn acoustic_profile_for_track(&self, track_id: i64) -> Result<Option<TrackAcousticProfile>> {
        let row = sqlx::query(
            "SELECT rms_mean, bass_mean, low_mid_mean, mid_mean, high_mid_mean, high_mean, \
                    spectral_centroid_mean, bpm_mean, bpm_variance, onset_mean, band_profile, \
                    frame_count \
             FROM track_acoustic_profiles \
             WHERE track_id = ?1",
        )
        .bind(track_id)
        .fetch_optional(self.pool())
        .await?;

Ok(row.map(|r| TrackAcousticProfile {
            track_id: r.get("track_id"),
            rms_mean: r.get("rms_mean"),
            bass_mean: r.get("bass_mean"),
            low_mid_mean: r.get("low_mid_mean"),
            mid_mean: r.get("mid_mean"),
            high_mid_mean: r.get("high_mid_mean"),
            high_mean: r.get("high_mean"),
            spectral_centroid_mean: r.get("spectral_centroid_mean"),
            bpm_mean: r.get("bpm_mean"),
            bpm_variance: r.get("bpm_variance"),
            onset_mean: r.get("onset_mean"),
            band_profile: {
                let s: String = r.get("band_profile");
                // Parse "[r1,r2,r3,r4,r5]" into [f32; 5]
                let s = s.trim();
                if s.starts_with('[') && s.ends_with(']') {
                    let inner = &s[1..s.len() - 1];
                    let mut arr = [0.0f32; 5];
                    let mut i = 0;
                    for part in inner.split(',') {
                        if i >= 5 { break; }
                        if let Ok(val) = part.trim().parse::<f32>() {
                            arr[i] = val;
                            i += 1;
                        }
                    }
                    arr
                } else {
                    [0.0f32; 5]
                }
            },
            frame_count: r.get("frame_count"),
        }))
    }

    /// Frecuencia y última escucha de cada track. La agregación ocurre en
    /// SQLite, no durante el render de la TUI.
    pub async fn listening_stats(&self) -> Result<Vec<TrackListeningStats>> {
        let rows = sqlx::query(
            "SELECT h.track_id, t.title, a.name AS artist_name, p.youtube_id, \
                    COUNT(h.id) AS play_count, MAX(h.played_at) AS last_played, \
                    MAX(h.played_at) >= datetime('now', '-7 days') AS recently_played \
             FROM history h \
             JOIN tracks t ON t.id = h.track_id \
             LEFT JOIN track_artists ta ON ta.track_id = t.id AND ta.position = 0 \
             LEFT JOIN artists a ON a.id = ta.artist_id \
             LEFT JOIN providers p ON p.track_id = t.id \
             GROUP BY h.track_id, t.title, a.name, p.youtube_id",
        )
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let track_id: i64 = r.get("track_id");
                let title: String = r.get("title");
                let artist: Option<String> = r.get("artist_name");
                let external: Option<String> = r.get("youtube_id");
                TrackListeningStats {
                    track_id,
                    key: external
                        .unwrap_or_else(|| format!("{}|{}", title, artist.clone().unwrap_or_default())),
                    artist_name: artist,
                    play_count: r.get("play_count"),
                    last_played: r.get("last_played"),
                    recently_played: r.get::<i64, _>("recently_played") != 0,
                }
            })
            .collect())
    }

    /// Búsqueda local por título de canción o nombre de artista. Incluye el id
    /// externo de YouTube para poder reproducir/enriquecer con recomendados.
    pub async fn search_local(&self, query: &str, limit: i64) -> Result<Vec<Track>> {
        let rows = sqlx::query(
            "SELECT t.id, t.title, t.duration, t.isrc, a.name AS artist_name, \
                    al.title AS album_title, p.youtube_id AS youtube_id \
             FROM tracks t \
             LEFT JOIN track_artists ta ON ta.track_id = t.id AND ta.position = 0 \
             LEFT JOIN artists a ON a.id = ta.artist_id \
             LEFT JOIN albums al ON al.id = t.album_id \
             LEFT JOIN providers p ON p.track_id = t.id \
             WHERE t.title LIKE '%' || ?1 || '%' \
                OR a.name LIKE '%' || ?1 || '%' \
             ORDER BY t.title LIMIT ?2",
        )
        .bind(query)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        Ok(rows.iter().map(row_to_track).collect())
    }

    // ------------------------------------------------------------- playlists

    /// Stream de audio persistido (URL de googlevideo) para `video_id`, si
    /// existe y no supera `max_age_secs` de antigüedad. La URL la re-verifica
    /// quien la consuma (GET rápido) antes de usarla: el TTL solo poda.
    pub async fn cached_stream_url(
        &self,
        video_id: &str,
        max_age_secs: i64,
    ) -> Result<Option<String>> {
        let url = sqlx::query_scalar::<_, String>(
            "SELECT url FROM stream_cache \
             WHERE video_id = ?1 AND created_at > datetime('now', '-' || ?2 || ' seconds')",
        )
        .bind(video_id)
        .bind(max_age_secs)
        .fetch_optional(self.pool())
        .await?;
        Ok(url)
    }

    /// Guarda (o refresca) el stream resuelto de un video para las siguientes
    /// reproducciones. Fallos de escritura no son críticos: la resolución
    /// normal sigue funcionando.
    pub async fn cache_stream(&self, video_id: &str, url: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO stream_cache (video_id, url, created_at) \
             VALUES (?1, ?2, datetime('now')) \
             ON CONFLICT(video_id) DO UPDATE \
             SET url = excluded.url, created_at = excluded.created_at",
        )
        .bind(video_id)
        .bind(url)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Elimina el stream persistido de un video (invalidación puntual).
    pub async fn delete_cached_stream(&self, video_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM stream_cache WHERE video_id = ?1")
            .bind(video_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Vacía la caché de streams (invalidación por proveedor).
    pub async fn clear_stream_cache(&self) -> Result<()> {
        sqlx::query("DELETE FROM stream_cache")
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Crea una playlist local. `Err` si ya existe un nombre igual.
    pub async fn create_playlist(&self, name: &str) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO playlists (name) VALUES (?1) \
             ON CONFLICT(name) DO NOTHING",
        )
        .bind(name)
        .execute(self.pool())
        .await?;
        if result.rows_affected() == 0 {
            anyhow::bail!("ya existe una playlist llamada «{name}»");
        }
        Ok(result.last_insert_rowid())
    }

    pub async fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        sqlx::query("UPDATE playlists SET name = ?2 WHERE id = ?1")
            .bind(id)
            .bind(name)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn delete_playlist(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM playlists WHERE id = ?1")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn list_playlists(&self) -> Result<Vec<PlaylistRow>> {
        let rows = sqlx::query(
            "SELECT p.id, p.name, p.created_at, \
                    (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id) AS track_count \
             FROM playlists p \
             ORDER BY p.name",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .iter()
            .map(|r| PlaylistRow {
                id: r.get("id"),
                name: r.get("name"),
                created_at: r.get("created_at"),
                track_count: r.get("track_count"),
            })
            .collect())
    }

    pub async fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<Track>> {
        let rows = sqlx::query(
            "SELECT t.id, t.title, t.duration, t.isrc, a.name AS artist_name, \
                    al.title AS album_title, p.youtube_id AS youtube_id \
             FROM playlist_tracks pt \
             JOIN tracks t ON t.id = pt.track_id \
             LEFT JOIN track_artists ta ON ta.track_id = t.id AND ta.position = 0 \
             LEFT JOIN artists a ON a.id = ta.artist_id \
             LEFT JOIN albums al ON al.id = t.album_id \
             LEFT JOIN providers p ON p.track_id = t.id \
             WHERE pt.playlist_id = ?1 \
             ORDER BY pt.position",
        )
        .bind(playlist_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.iter().map(row_to_track).collect())
    }

    pub async fn add_to_playlist(&self, playlist_id: i64, track_id: i64) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) \
             VALUES (?1, ?2, \
                (SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1))",
        )
        .bind(playlist_id)
        .bind(track_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn remove_from_playlist(&self, playlist_id: i64, track_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2")
            .bind(playlist_id)
            .bind(track_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    // ------------------------------------------------------- portada override

    /// Guarda una imagen de portada local que reemplaza a la de YouTube.
    pub async fn set_artwork_override(&self, track_id: i64, image: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO artwork_overrides (track_id, image, updated_at) \
             VALUES (?1, ?2, datetime('now')) \
             ON CONFLICT(track_id) DO UPDATE SET image = excluded.image, \
               updated_at = excluded.updated_at",
        )
        .bind(track_id)
        .bind(image)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn get_artwork_override(&self, track_id: i64) -> Result<Option<String>> {
        let row = sqlx::query("SELECT image FROM artwork_overrides WHERE track_id = ?1")
            .bind(track_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|r| r.get("image")))
    }

    // ------------------------------------------------------------- letras

    /// Id interno canónico de la canción que tiene este `external_id`
    /// (`youtube_id`). `None` si la canción aún no se guardó. Permite buscar en
    /// la caché de letras aunque el track en mano llegara sin su `id` interno
    /// (p. ej. recién buscado y reproducido por primera vez).
    pub async fn internal_id_for_external(&self, external_id: &str) -> Result<Option<i64>> {
        let row = sqlx::query("SELECT track_id FROM providers WHERE youtube_id = ?1")
            .bind(external_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|r| r.get("track_id")))
    }

    /// Guarda una letra sincronizada (LRC) cacheada de un track (LRCLIB). La
    /// caché de letras solo guarda sincronizadas: la letra plana heredada no
    /// tiene consumidores y no se escribe (ni se sirve) como karaoke.
    pub async fn cache_synced_lyrics(&self, track_id: i64, body: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO lyrics_cache (track_id, body, footer, synced, fetched_at) \
             VALUES (?1, ?2, NULL, 1, datetime('now')) \
             ON CONFLICT(track_id) DO UPDATE SET body = excluded.body, \
               footer = NULL, synced = 1, fetched_at = excluded.fetched_at",
        )
        .bind(track_id)
        .bind(body)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Letra sincronizada (LRC) cacheada de un track, si existe. Las filas
    /// planas (legado de la implementación antigua) no se devuelven.
    pub async fn get_synced_lyrics(&self, track_id: i64) -> Result<Option<String>> {
        let row = sqlx::query("SELECT body FROM lyrics_cache WHERE track_id = ?1 AND synced = 1")
            .bind(track_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|r| r.get("body")))
    }
}

async fn upsert_track_inner(
    tx: &mut Transaction<'_, Sqlite>,
    track: &Track,
    provider_ids: &HashMap<Source, String>,
) -> Result<i64> {
    // El `youtube_id` es la identidad canónica de una canción. Antes se
    // insertaba siempre y una segunda reproducción chocaba con el UNIQUE de
    // providers; por ello no había frecuencia persistente fiable.
    if let Some(youtube_id) = provider_ids.get(&Source::YouTube) {
        if let Some(row) = sqlx::query("SELECT track_id FROM providers WHERE youtube_id = ?1")
            .bind(youtube_id)
            .fetch_optional(&mut **tx)
            .await?
        {
            let track_id: i64 = row.get("track_id");
            sqlx::query(
                "UPDATE tracks SET duration = COALESCE(?2, duration), \
                 isrc = COALESCE(?3, isrc) WHERE id = ?1",
            )
            .bind(track_id)
            .bind(track.duration.map(|d| d.as_millis() as i64))
            .bind(&track.isrc)
            .execute(&mut **tx)
            .await?;
            return Ok(track_id);
        }
    }
    let album_id = match &track.album {
        Some(album) => Some(get_or_create_album(tx, album).await?),
        None => None,
    };

    let mut artist_ids = Vec::new();
    for artist in &track.artists {
        artist_ids.push(get_or_create_artist(tx, artist).await?);
    }

    let result =
        sqlx::query("INSERT INTO tracks (title, duration, isrc, album_id) VALUES (?1, ?2, ?3, ?4)")
            .bind(&track.title)
            .bind(track.duration.map(|d| d.as_millis() as i64))
            .bind(&track.isrc)
            .bind(album_id)
            .execute(&mut **tx)
            .await?;
    let track_id = result.last_insert_rowid();

    for (position, artist_id) in artist_ids.into_iter().enumerate() {
        sqlx::query(
            "INSERT OR IGNORE INTO track_artists (track_id, artist_id, position) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(track_id)
        .bind(artist_id)
        .bind(position as i64)
        .execute(&mut **tx)
        .await?;
    }

    for genre in &track.genres {
        let genre_id = get_or_create_genre(tx, &genre.name).await?;
        sqlx::query("INSERT OR IGNORE INTO track_genres (track_id, genre_id) VALUES (?1, ?2)")
            .bind(track_id)
            .bind(genre_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO tags (track_id, name, source) VALUES (?1, ?2, ?3)")
            .bind(track_id)
            .bind(&genre.name)
            .bind(track.source.as_str())
            .execute(&mut **tx)
            .await?;
    }

    upsert_provider(tx, track_id, provider_ids).await?;

    Ok(track_id)
}

async fn get_or_create_album(tx: &mut Transaction<'_, Sqlite>, album: &Album) -> Result<i64> {
    let release_date = album.release_date.map(|d| d.format("%Y-%m-%d").to_string());

    if let Some(row) = sqlx::query(
        "SELECT id FROM albums \
         WHERE title = ?1 AND COALESCE(release_date, '') = COALESCE(?2, '') LIMIT 1",
    )
    .bind(&album.title)
    .bind(release_date.as_deref())
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(row.get("id"));
    }

    let result: SqliteQueryResult = sqlx::query(
        "INSERT INTO albums (title, release_date, cover, label) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&album.title)
    .bind(release_date.as_deref())
    .bind(&album.cover)
    .bind(&album.label)
    .execute(&mut **tx)
    .await?;
    Ok(result.last_insert_rowid())
}

async fn get_or_create_artist(tx: &mut Transaction<'_, Sqlite>, artist: &Artist) -> Result<i64> {
    if let Some(row) = sqlx::query("SELECT id FROM artists WHERE name = ?1 LIMIT 1")
        .bind(&artist.name)
        .fetch_optional(&mut **tx)
        .await?
    {
        return Ok(row.get("id"));
    }

    let result: SqliteQueryResult = sqlx::query(
        "INSERT INTO artists (name, country, biography, image) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&artist.name)
    .bind(&artist.country)
    .bind(&artist.biography)
    .bind(&artist.image)
    .execute(&mut **tx)
    .await?;
    Ok(result.last_insert_rowid())
}

async fn get_or_create_genre(tx: &mut Transaction<'_, Sqlite>, name: &str) -> Result<i64> {
    let normalized = name.trim().to_lowercase();
    if let Some(row) = sqlx::query("SELECT id FROM genres WHERE name = ?1 LIMIT 1")
        .bind(&normalized)
        .fetch_optional(&mut **tx)
        .await?
    {
        return Ok(row.get("id"));
    }

    let result: SqliteQueryResult = sqlx::query("INSERT INTO genres (name) VALUES (?1)")
        .bind(&normalized)
        .execute(&mut **tx)
        .await?;
    Ok(result.last_insert_rowid())
}

async fn upsert_provider(
    tx: &mut Transaction<'_, Sqlite>,
    track_id: i64,
    provider_ids: &HashMap<Source, String>,
) -> Result<()> {
    let youtube_id = provider_ids.get(&Source::YouTube).cloned();

    sqlx::query(
        "INSERT INTO providers (track_id, youtube_id) \
         VALUES (?1, ?2) \
         ON CONFLICT(track_id) DO UPDATE SET \
           youtube_id = COALESCE(excluded.youtube_id, providers.youtube_id)",
    )
    .bind(track_id)
    .bind(youtube_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn row_to_track(row: &sqlx::sqlite::SqliteRow) -> Track {
    let mut artist = Artist::new(
        row.get::<Option<String>, _>("artist_name")
            .unwrap_or_default(),
        None,
        None,
        None,
    );
    if artist.name.is_empty() {
        artist = Artist::new("Unknown".to_string(), None, None, None);
    }
    let mut track = Track::new(row.get("title"), vec![artist], Source::YouTube);
    track.id = row.get("id");
    track.duration = row
        .get::<Option<i64>, _>("duration")
        .map(|ms| std::time::Duration::from_millis(ms as u64));
    track.isrc = row.get("isrc");
    track.external_id = row.get::<Option<String>, _>("youtube_id");
    track.album = row
        .get::<Option<String>, _>("album_title")
        .map(|title| Album::new(title, None, None, None));
    track
}

// ---------------------------------------------------------------------
// Caché fría de resoluciones: adaptador SQLite del puerto
// `media::ResolutionCache` (DIP: Infraestructura implementa el puerto).
// ---------------------------------------------------------------------

/// Antigüedad máxima de una URL persistida. La URL se re-verifica en vivo
/// antes de usarse (validador del resolver); este TTL solo poda entradas
/// viejas: las URLs de googlevideo viven horas.
const STREAM_CACHE_MAX_AGE_SECS: i64 = 6 * 60 * 60;

/// Capa FRÍA de la caché de resoluciones sobre SQLite.
///
/// Solo persiste la URI: sus entradas nacen "desnudas" (sin cabeceras ni
/// metadatos) y el validador en vivo del resolver debe repararlas antes de
/// servirse. La poda es perezosa, por edad al leer; `expiring_within` devuelve
/// vacío por diseño.
#[derive(Debug, Clone)]
pub struct DbResolutionCache {
    db: Db,
}

impl DbResolutionCache {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl crate::media::ResolutionCache for DbResolutionCache {
    async fn get(&self, key: &str) -> Option<crate::domain::stream::StreamResolution> {
        // Solo las claves tipo video_id tienen sentido aquí; el identificador
        // compuesto de fallback ("título|artista") nunca provino de la red.
        if key.contains('|') {
            return None;
        }
        let url = self
            .db
            .cached_stream_url(key, STREAM_CACHE_MAX_AGE_SECS)
            .await
            .ok()
            .flatten()?;
        Some(crate::domain::stream::StreamResolution::new(
            Source::YouTube,
            url,
        ))
    }

    async fn put(&self, key: &str, resolution: crate::domain::stream::StreamResolution) {
        if !key.contains('|') {
            let _ = self.db.cache_stream(key, &resolution.uri).await;
        }
    }

    async fn invalidate(&self, key: &str) {
        if !key.contains('|') {
            let _ = self.db.delete_cached_stream(key).await;
        }
    }

    async fn clear_provider(&self, _source: Source) {
        let _ = self.db.clear_stream_cache().await;
    }

    async fn expiring_within(
        &self,
        _window: std::time::Duration,
    ) -> Vec<(String, chrono::DateTime<chrono::Utc>)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::domain::{artist::Artist, genre::Genre, source::Source, track::Track};

    use super::*;

    #[tokio::test]
    async fn upsert_and_history_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("music.db");
        let db = Db::connect(db_path.to_str().unwrap()).await.unwrap();

        let mut track = Track::new(
            "Test Song".to_string(),
            vec![Artist::new("Test Artist".to_string(), None, None, None)],
            Source::YouTube,
        );
        track.duration = Some(std::time::Duration::from_secs(200));
        track.genres = vec![Genre::new("rock".to_string())];
        track.external_id = Some("dQw4w9WgXcQ".to_string());
        track.isrc = Some("USX000000000".to_string());

        let mut ids = HashMap::new();
        ids.insert(Source::YouTube, "dQw4w9WgXcQ".to_string());
        let id = db.upsert_track(&track, &ids).await.unwrap();
        assert!(id > 0);

        db.record_history(id, Source::YouTube, Some(200_000))
            .await
            .unwrap();
        // La misma canción conserva su id canónico y acumula frecuencia.
        assert_eq!(db.upsert_track(&track, &ids).await.unwrap(), id);
        db.record_history(id, Source::YouTube, Some(200_000))
            .await
            .unwrap();

        let history = db.recent_history(10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].title, "Test Song");
        assert_eq!(history[0].artist_name.as_deref(), Some("Test Artist"));
        assert_eq!(history[0].source, Source::YouTube);
        assert_eq!(history[0].play_count, 2);

        let stats = db.listening_stats().await.unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].key, "dQw4w9WgXcQ");
        assert_eq!(stats[0].play_count, 2);
        assert!(stats[0].recently_played);

        let local = db.search_local("Test", 10).await.unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].primary_artist_name(), Some("Test Artist"));

        let tags: Vec<(String, String)> =
            sqlx::query_as("SELECT name, source FROM tags WHERE track_id = ?1")
                .bind(id)
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(tags, vec![("rock".to_string(), "youtube".to_string())]);
    }

    #[tokio::test]
    async fn playlist_crud() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::connect(dir.path().join("music.db").to_str().unwrap())
            .await
            .unwrap();

        let mut track = Track::new(
            "A".to_string(),
            vec![Artist::new("X".to_string(), None, None, None)],
            Source::YouTube,
        );
        track.external_id = Some("vid-a".to_string());
        let id = db.upsert_track(&track, &HashMap::new()).await.unwrap();

        let pl = db.create_playlist("Mi lista").await.unwrap();
        db.add_to_playlist(pl, id).await.unwrap();
        db.add_to_playlist(pl, id).await.unwrap(); // dedupe

        let pls = db.list_playlists().await.unwrap();
        assert_eq!(pls.len(), 1);
        assert_eq!(pls[0].track_count, 1);
        assert_eq!(pls[0].name, "Mi lista");

        let tracks = db.playlist_tracks(pl).await.unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "A");

        db.remove_from_playlist(pl, id).await.unwrap();
        assert_eq!(db.playlist_tracks(pl).await.unwrap().len(), 0);

        db.set_artwork_override(id, "file:///tmp/c.jpeg")
            .await
            .unwrap();
        assert_eq!(
            db.get_artwork_override(id).await.unwrap().as_deref(),
            Some("file:///tmp/c.jpeg")
        );

        // Las letras sincronizadas (LRC) se cachean con el flag de sincronía y
        // solo esas se sirven como fuente del karaoke.
        db.cache_synced_lyrics(id, "[00:01.00] letra\n[00:05.00] letra")
            .await
            .unwrap();
        assert_eq!(
            db.get_synced_lyrics(id).await.unwrap().as_deref(),
            Some("[00:01.00] letra\n[00:05.00] letra")
        );

        // Una letra plana heredada (legado sin consumidor) no se sirve como
        // sincronizada: la caché respeta la distinción por el flag `synced`.
        let legacy = Track::new(
            "C".to_string(),
            vec![Artist::new("Z".to_string(), None, None, None)],
            Source::YouTube,
        );
        let mut legacy_ids = HashMap::new();
        legacy_ids.insert(Source::YouTube, "vid-c".to_string());
        let legacy_id = db.upsert_track(&legacy, &legacy_ids).await.unwrap();
        sqlx::query(
            "INSERT INTO lyrics_cache (track_id, body, footer, synced, fetched_at) \
             VALUES (?1, ?2, NULL, 0, datetime('now'))",
        )
        .bind(legacy_id)
        .bind("letra plana vieja")
        .execute(db.pool())
        .await
        .unwrap();
        assert!(
            db.get_synced_lyrics(legacy_id).await.unwrap().is_none(),
            "la letra plana no se devuelve como sincronizada"
        );
    }

    #[tokio::test]
    async fn internal_id_for_external() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::connect(dir.path().join("music.db").to_str().unwrap())
            .await
            .unwrap();

        let mut track = Track::new(
            "B".to_string(),
            vec![Artist::new("Y".to_string(), None, None, None)],
            Source::YouTube,
        );
        track.external_id = Some("vid-b".to_string());
        let mut ids = HashMap::new();
        ids.insert(Source::YouTube, "vid-b".to_string());
        let id = db.upsert_track(&track, &ids).await.unwrap();

        // Resuelve el id interno a partir del youtube_id, aunque no se tenga el
        // Track (p. ej. al reproducir en vivo un resultado de búsqueda).
        assert_eq!(
            db.internal_id_for_external("vid-b").await.unwrap(),
            Some(id)
        );
        assert_eq!(
            db.internal_id_for_external("no-existe").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn stream_cache_roundtrip_and_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::connect(dir.path().join("music.db").to_str().unwrap())
            .await
            .unwrap();

        // Vacío al principio.
        assert_eq!(
            db.cached_stream_url("vid-a", 3600).await.unwrap(),
            None,
            "sin caché no hay stream"
        );

        // Guarda y recupera.
        db.cache_stream("vid-a", "https://googlevideo/stream-a")
            .await
            .unwrap();
        assert_eq!(
            db.cached_stream_url("vid-a", 3600).await.unwrap(),
            Some("https://googlevideo/stream-a".to_string())
        );

        // Actualiza la URL del mismo video (los streams expiran y se re-resuelven).
        db.cache_stream("vid-a", "https://googlevideo/stream-a2")
            .await
            .unwrap();
        assert_eq!(
            db.cached_stream_url("vid-a", 3600).await.unwrap(),
            Some("https://googlevideo/stream-a2".to_string())
        );

        // El TTL poda: 0 segundos de antigüedad máxima → nada.
        assert_eq!(
            db.cached_stream_url("vid-a", 0).await.unwrap(),
            None,
            "el TTL filtra entradas viejas"
        );
    }
}
