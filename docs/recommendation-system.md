# FASE 8–10 — Sistema de recomendaciones local

> **Fuente única de verdad**: este documento describe lo que existe y lo que se diseña.
> No se inventan features inexistentes.

---

## 1. Inventario de features acústicas REALES

### 1.1 Fuente: `src/analysis/features.rs` — `AudioFeatures` (smooth, 0..1 salvo BPM)

| Campo | Tipo | Descripción |
|---|---|---|
| `rms` | `f32` | Amplitud RMS suavizada (energía global) |
| `amplitude` | `f32` | Amplitud pico suavizada |
| `bass` | `f32` | Energía relativa banda 20–250 Hz |
| `low_mid` | `f32` | Energía relativa banda 250–500 Hz |
| `mid` | `f32` | Energía relativa banda 500–2000 Hz |
| `high_mid` | `f32` | Energía relativa banda 2000–4000 Hz |
| `high` | `f32` | Energía relativa banda 4000–16000 Hz |
| `spectral_centroid` | `f32` | Centroide espectral normalizado (0=graves, 1=agudos) |
| `spectral_flux` | `f32` | Flujo espectral suavizado (temporalidad del cambio espectral) |
| `onset` | `f32` | Fuerza del onset en este frame (pico crudo) |
| `beat` | `bool` | Pulso de beat discreto en este frame |
| `beat_confidence` | `f32` | Confianza de detección de beat (0..1) |
| `bpm` | `f32` | Tempo estimado en BPM (0 = desconocido) |

### 1.2 Fuente: `src/analysis/bands.rs` — `BandRatios` (raw)

Las mismas 5 bandas (`bass`, `low_mid`, `mid`, `high_mid`, `high`) como raw, sin suavizar. Bordes: `[20, 250, 500, 2000, 4000, 16000]` Hz.

### 1.3 Fuente: `src/analysis/engine.rs` — pipeline DSP

FFT de 2048 muestras, hop de 512 (~86 fps). El pipeline produce:
- `RawFeatures`: `rms`, `amplitude`, `bands`, `centroid_norm`, `flux`
- Suavizado EMA ataque/release separado → `AudioFeatures`
- `BpmEstimator` por autocorrelación del envelope de onsets (60–200 BPM)
- `OnsetDetector` con flujo espectral + umbral adaptativo

### 1.4 Lo que **NO existe** (no se inventa)

- ❌ Zero crossing rate
- ❌ Spectral rolloff
- ❌ MFCC
- ❌ Chroma / tonal features
- ❌ Harmonic/noise ratio
- ❌ Loudness (LUFS)
- ❌ Brightness / roughness / warmth como nombres dedicados

### 1.5 Features derivables de lo que existe

| Feature derivada | Cálculo | Significado |
|---|---|---|
| **Energy** | `rms` | Energía global de la señal |
| **Tempo** | `bpm` | BPM estimado |
| **Brightness** | `spectral_centroid` | Cuán agudo es el espectro |
| **Spectral spread** | `high + high_mid - bass - low_mid` | Dispersión de energía |
| **Bass content** | `bass` | Energía en graves |
| **High content** | `high` | Energía en agudos |
| **Dynamic range** | `amplitude / (rms + ε)` | Relación pico/RMS |
| **Attack/activity** | `onset` + `spectral_flux` | Carácter percusivo / variabilidad |
| **Band energy profile** | `[bass, low_mid, mid, high_mid, high]` | Firma espectral de 5 dimensiones |

---

## 2. Inventario de metadata REAL

### 2.1 Estructuras de dominio (`src/domain/`)

**`Track`** (`track.rs`):
- `id`, `title`, `artists: Vec<Artist>`, `album: Option<Album>`, `duration: Option<Duration>`, `genres: Vec<Genre>`, `source: Source`, `external_id: Option<String>`, `isrc: Option<String>`, `url: Option<String>`, `thumbnail: Option<Thumbnail>`

**`Artist`** (`artist.rs`):
- `id`, `name`, `country`, `biography`, `image`, `genres: Vec<String>`, `external_id`, `total_duration`

**`Album`** (`album.rs`):
- `id`, `title`, `release_date`, `cover`, `label`, `artist_ids`

**`Genre`** (`genre.rs`):
- `id`, `name`

**`Source`** (`source.rs`): `YouTube` (único origen).

### 2.2 Tablas SQLite (migrations)

| Tabla | Campos relevantes |
|---|---|
| `tracks` | id, title, duration, isrc, album_id |
| `artists` | id, name, country, biography, image |
| `albums` | id, title, release_date, cover, label |
| `genres` | id, name |
| `track_artists` | track_id, artist_id, position |
| `track_genres` | track_id, genre_id |
| `artist_genres` | artist_id, genre_id |
| `providers` | track_id, youtube_id |
| `tags` | id, track_id, name, source — **tags = nombres de géneros actualmente** |
| `history` | id, track_id, played_at, source, duration |
| `playlists` | id, name, created_at |
| `playlist_tracks` | playlist_id, track_id, position |
| `artwork_overrides` | track_id, image |
| `lyrics_cache` | track_id, body, synced |
| `stream_cache` | video_id, url |

### 2.3 Qué metadata está disponible para recomendaciones

- ✅ artist (vía `artists`, `track_artists`)
- ✅ album (vía `album`, `album_id`)
- ✅ genre (vía `genres`, `track_genres`)
- ✅ tags (vía `tags` — actualmente = géneros)
- ✅ provider (`Source::YouTube`)
- ✅ track relationships (`track_artists` para co-artistas, `track_genres` para co-géneros)
- ✅ play count (SQL `COUNT(h.id) OVER (PARTITION BY h.track_id)`)
- ✅ recent plays (SQL `MAX(h.played_at) >= datetime('now', '-7 days')`)
- ✅ last played (SQL `MAX(h.played_at)`)

---

## 3. Inventario de historial de reproducción REAL

### 3.1 Qué existe (`src/app/history.rs`, `src/infrastructure/storage.rs`)

**`History::record(track_id, source, duration)`** — registra un evento de reproducción.

**`History::recent(limit)`** — últimas N reproducciones con `play_count`.

**`History::stats()`** — agregado por track: `play_count`, `last_played`, `recently_played` (7 días).

### 3.2 Tabla `history`

```sql
CREATE TABLE history (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id  INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    played_at TEXT    NOT NULL DEFAULT (datetime('now')),
    source    TEXT    NOT NULL,
    duration  INTEGER  -- duración de la canción (ms)
);
```

### 3.3 Lo que **NO existe** como eventos separados

- ❌ Evento explícito de "play completado" vs "play abandonado"
- ❌ Evento de "skip" (adelantar)
- ❌ Evento de "like" / "dislike"
- ❌ Evento de "replay" explícito
- ❌ Evento de "seek" para posición de escucha

### 3.4 Señales implícitas derivables del estado actual

| Señal | Cómo derivarla | Significado |
|---|---|---|
| **Play** | `record(track_id)` en `history` | El usuario comenzó a escuchar |
| **Completado** | `history.duration ≈ track.duration` | Terminó de escuchar |
| **Skip** | `history.duration < threshold` (p. ej. < 30s) | Abandonó rápidamente |
| **Replay** | Misma canción aparece múltiples veces en historial | Preferencia explícita |
| **Recencia** | `played_at` reciente | Interes actual |
| **Frecuencia** | `COUNT(*)` por track | Preferencia sostenida |

---

## 4. Arquitectura del sistema de recomendaciones (FASE 8–10)

### 4.1 Ubicación en el proyecto

```
src/
├── recommendation/
│   ├── mod.rs
│   ├── types.rs          # FeatureVector, UserProfile, Candidate, RecommendationScore
│   ├── metadata_score.rs # Similaridad metadata
│   ├── acoustic_score.rs # Similaridad acústica
│   ├── affinity_score.rs # Afinidad usuario/historial
│   ├── recency_score.rs  # Ajuste recencia
│   ├── popularity_score.rs# Ajuste popularidad
│   ├── negative_score.rs # Señales negativas
│   ├── ranker.rs         # Pipeline de scoring
│   └── profile.rs        # Perfil musical local
src/
├── domain/
│   └── recommendation.rs # Dominio: RecommendationCandidate, UserTasteProfile
infrastructure/
│   └── storage.rs        # Nuevas queries para recomendaciones
```

### 4.2 Flujo del pipeline

```
Catálogo de tracks (todos los tracks conocidos)
        ↓
Generación de candidatos (top-N por popularidad o aleatorio)
        ↓
┌───────────────────────────────────────────────────────┐
│  Para cada canción candidata, calcular:               │
│                                                       │
│  metadata_similarity(track, user_profile)    ∈ [0,1]  │
│       ↓                                           +   │
│  acoustic_similarity(track, user_profile)     ∈ [0,1]  │
│       ↓                                           +   │
│  user_affinity(track, history)               ∈ [0,1]  │
│       ↓                                           +   │
│  recency_bonus(track, history)               ∈ [0,1]  │
│       ↓                                           +   │
│  popularity_factor(track, history)           ∈ [0,1]  │
│       ↓                                           +   │
│  negative_penalty(track, history)            ∈ [0,1]  │
│       ↓                                           +   │
│  final_score = w₁·meta + w₂·acoustic + w₃·affinity   │
│              + w₄·recency + w₅·popularity - w₆·neg    │
└───────────────────────────────────────────────────────┘
        ↓
Ranking por final_score descendente
        ↓
Top-N recomendaciones
```

### 4.3 Pesos (sin constantes arbitrarias)

Los pesos se normalizan para que cada componente esté en [0,1] y se expresen como proporciones que suman 1:

| Componente | Peso | Justificación |
|---|---|---|
| `w_meta` | 0.20 | La metadata (género, artista) es la señal más fiable y barata |
| `w_acoustic` | 0.30 | El sonido es el corazón del karaoke; emparejar el perfil acústico es clave |
| `w_affinity` | 0.15 | Lo que el usuario ya escuchó es una señal, pero no la dominante |
| `w_recency` | 0.25 | La recencia tiene peso comparable al acoustic: el gusto cambia |
| `w_popularity` | 0.10 | Evita que el recomendador sea demasiado elitista; peso moderado |

Pesos totales de suma: `w_meta + w_acoustic + w_affinity + w_recency + w_popularity = 1.00`
`w_negative` es una penalización multiplicativa (0.5×) o aditiva (-0.3).

---

## 5. Componentes detallados del scoring

### 5.1 `metadata_similarity(track, profile) → f32`

Combina varias señales de metadata:

```
meta_sim = 0.35 · artist_match   + 0.25 · genre_match
         + 0.20 · album_match     + 0.10 · tag_match
         + 0.10 · decade_match
```

- **artist_match**: proporción de artistas del track que están en el perfil de artistas del usuario
- **genre_match**: proporción de géneros que coinciden con el perfil de géneros
- **album_match**: 1.0 si el álbum está en el historial, 0.0 en caso contrario
- **tag_match**: similar a genre_match sobre tags
- **decade_match**: 1.0 si el año de lanzamiento está en la misma década que las canciones favoritas del usuario

### 5.2 `acoustic_similarity(track_a, track_b) → f32`

Dado que las `AudioFeatures` se generan por frame durante el análisis, se necesita un **vector de features por track** que se calcula una vez (al analizar el track o al promediar sus frames).

**Feature vector por track** (promedio de frames, normalizado 0..1):

```rust
struct TrackAcousticProfile {
    rms_mean: f32,
    bass_mean: f32,
    low_mid_mean: f32,
    mid_mean: f32,
    high_mid_mean: f32,
    high_mean: f32,
    spectral_centroid_mean: f32,
    bpm_mean: f32,          // BPM estable del track
    onset_mean: f32,        // Actividad de onset promedio
    band_profile: [f32; 5], // [bass, low_mid, mid, high_mid, high]
}
```

**Similaridad**: distancia coseno entre vectores normalizados:

```
acoustic_sim = 1 - cosine_distance(vec_a, vec_b)
```

Donde el vector de cada track es:
```
[rms, bass, low_mid, mid, high_mid, high, centroid, bpm_norm, onset]
```
con `bpm_norm = bpm / 200.0` (BPM máximo razonable).

### 5.3 `user_affinity(track, history) → f32`

Afinidad basada en el historial de reproducción del usuario:

```
affinity = 0.4 · artist_affinity
         + 0.3 · genre_affinity
         + 0.2 · acoustic_affinity  // contra el perfil acústico del usuario
         + 0.1 · direct_match       // mismo track ya escuchado y completado
```

- **artist_affinity**: `count_plays(artist) / total_plays`
- **genre_affinity**: `count_plays(genre) / total_plays`
- **acoustic_affinity**: similitud promedio de features entre este track y los tracks completados del usuario
- **direct_match**: 1.0 si ya se completó esta canción, 0.0 si nunca se escuchó, penalización si se saltó

### 5.4 `recency_bonus(track, history) → f32`

Decaimiento exponencial basado en días desde la última escucha:

```
days_since = now - last_played
recency = e^(-λ · days_since)    // λ = ln(2) / 14  (media: 14 días)
```

Si nunca se escuchó: `recency = 0.0` (sin bonus).

### 5.5 `popularity_factor(track, history) → f32`

Popularidad normalizada por decil:

```
popularity = log1p(play_count) / log1p(max_play_count)
```

Esto evita que los tracks con 10000 reproducciones dominen sobre los que tienen 100, pero mantiene una señal de popularidad moderada. El peso es bajo (0.05) para que no domine el ranking.

### 5.6 `negative_penalty(track, history) → f32`

Penalización multiplicativa para señales negativas:

```
penalty = 1.0
if skip_rate(track) > 0.5:   penalty *= 0.3   // Muchos skips → evitar
if skip_rate(track) > 0.2:   penalty *= 0.7   // Skips moderados → penalizar
```

**Importante**: `skip` NO es igual a `dislike`. Un skip se detecta cuando `history.duration < track.duration * 0.2` (se escuchó menos del 20% de la canción). No se deduce desagrado explícito, solo abandono temprano.

### 5.7 `RecommendationScore` final

```rust
struct RecommendationScore {
    track_id: i64,
    final_score: f64,            // 0..1 (o logit, normalizado)
    components: ScoreComponents {
        metadata: f64,           // 0..1
        acoustic: f64,           // 0..1
        affinity: f64,           // 0..1
        recency: f64,            // 0..1
        popularity: f64,         // 0..1
        negative: f64,           // 0..1 (penalización, < 1.0)
    },
}
```

```
final_score = (w_meta·meta + w_acoustic·acoustic + w_affinity·affinity
             + w_recency·recency + w_popularity·popularity) * negative
```

---

## 6. Perfil musical local (`UserTasteProfile`) — FASE 10

### 6.1 Diseño

El perfil se deriva **exclusivamente del historial**. No requiere edición manual.

```rust
#[derive(Debug, Clone, Default)]
pub struct UserTasteProfile {
    // ── Metadata ──
    pub favorite_artists: Vec<String>,          // Artistas más escuchados
    pub favorite_genres: Vec<String>,           // Géneros más escuchados
    pub favorite_albums: Vec<i64>,              // Álbumes completados
    pub favorite_decades: Vec<i64>,             // Décadas con más escuchas
    pub favorite_tags: Vec<String>,             // Tags frecuentes

    // ── Features acústicos (promedio ponderado por completion_rate) ──
    pub acoustic_profile: AcousticProfile,

    // ── Historial de señales ──
    pub total_plays: u64,
    pub total_skips: u64,
    pub total_completions: u64,
    pub tracks_played: HashSet<i64>,
    pub tracks_completed: HashSet<i64>,
    pub tracks_skipped: HashSet<i64>,
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
    /// Perfil de bandas como array
    pub band_profile: [f32; 5],
}
```

### 6.2 Cómo se construye

Se itera sobre el historial agregado:

```rust
impl UserTasteProfile {
    fn from_history(history: &[TrackPlayEvent], tracks: &[Track]) -> Self {
        let mut profile = UserTasteProfile::default();

        for event in history {
            // Señales de artista, género, álbum
            profile.favorite_artists.push(artist);
            profile.favorite_genres.push(genre);
            // ... etc.

            // Perfil acústico: ponderado por completion_rate
            // Completado = peso 1.0, Abandonado = peso 0.1
            let weight = if event.completed { 1.0 } else { 0.1 };
            profile.acoustic_profile.rms     += weight * features.rms;
            profile.acoustic_profile.bass     += weight * features.bass;
            // ... etc.
        }

        // Promediar
        let total_weight: f32 = ...;
        profile.acoustic_profile.rms /= total_weight;
        // ...

        profile.total_plays = history.len() as u64;
        profile.total_skips = history.iter().filter(|e| e.skipped).count() as u64;
        profile.total_completions = history.iter().filter(|e| e.completed).count() as u64;
    }
}
```

### 6.3 Eventos de escucha y señales

| Evento | Señal | Peso en perfil | Significado |
|---|---|---|---|
| **Play** | `play_count += 1` | Bajo (0.1) | Interés inicial |
| **Completion** (duración ≈ duración del track) | `completion_count += 1` | Alto (1.0) | Preferencia fuerte |
| **Skip** (duración < 20% del track) | `skip_count += 1` | Negativo (0.0) | Evitar; NO es dislike |
| **Replay** (misma canción múltiples veces) | `replay_count += 1` | Alto (1.5) | Preferencia explícita |

**Reglas clave**:
- `play ≠ like`: un play puede ser curiosidad o autoplay. Solo completion y replay son señales fuertes.
- `skip ≠ dislike`: un skip puede ser error de click o cola equivocada. Solo skip repetido cuenta.
- `completion ≠ love`: completar una canción puede ser costumbre, no preferencia explícita.

---

## 7. Implementación propuesta

### 7.1 Nuevos archivos

```
src/recommendation/
├── mod.rs
├── types.rs
├── profile.rs
├── scoring/
│   ├── mod.rs
│   ├── metadata.rs
│   ├── acoustic.rs
│   ├── affinity.rs
│   ├── recency.rs
│   ├── popularity.rs
│   └── negative.rs
└── ranker.rs
```

### 7.2 Nuevas queries en `src/infrastructure/storage.rs`

```rust
// Obtener todos los tracks con sus features (para generación de candidatos)
pub async fn all_tracks() -> Result<Vec<TrackWithFeatures>>

// Obtener eventos de historial con contexto de completion
pub async fn history_with_completion() -> Result<Vec<PlaybackEvent>>

// Obtener estadísticas por artista, género, década
pub async fn artist_play_counts() -> Result<Vec<ArtistPlayCount>>
pub async fn genre_play_counts() -> Result<Vec<GenrePlayCount>>
```

### 7.3 Nuevo schema (migración futura, FASE 10+)

```sql
-- Perfiles acústicos por track (calculados por el análisis, no por usuario)
CREATE TABLE track_acoustic_profiles (
    track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    rms_mean REAL,
    bass_mean REAL,
    low_mid_mean REAL,
    mid_mean REAL,
    high_mid_mean REAL,
    high_mean REAL,
    spectral_centroid_mean REAL,
    bpm_mean REAL,
    onset_mean REAL,
    band_profile TEXT,  -- JSON [bass, low_mid, mid, high_mid, high]
    frame_count INTEGER DEFAULT 0
);
```

### 7.4 Orden de implementación

1. **FASE 8**: `TrackAcousticProfile` — almacenar features promediadas por track. Nueva tabla `track_acoustic_profiles` + query para obtenerla + cálculo desde las features del análisis.
2. **FASE 9**: `RecommendationScore`, componentes de scoring, `ranker`. Sin `UserTasteProfile` inicial: usa `play_count` y `last_played` como proxy simple.
3. **FASE 10**: `UserTasteProfile` completo, `from_history()`, integración de todos los componentes de scoring con el perfil.

---

## 8. Diferencia: popular vs recomendado para este usuario

| Aspecto | Popular | Recomendado para este usuario |
|---|---|---|
| Criterio | `play_count` global | `RecommendationScore` personalizado |
| Fuente de datos | Agregado de todos los usuarios | Solo el historial de este usuario |
| Sesgo | Populares siempre arriba | Se adapta al gusto individual |
| Uso | "Más escuchadas", "Trending" | "Para ti", "Basado en tu historial" |
| Composición | Popularidad pura | metadata + acoustic + affinity + recency + popularity - negatives |

Una canción puede ser la más popular y aun así ser una mala recomendación si no coincide con el perfil acústico ni de metadata del usuario. El componente de popularidad (`w_popularity = 0.05`) es un suavizador, no el motor principal.

---

## 9. Resumen de lo que existe vs lo que se diseña

### ✅ Existe y se reutiliza
- `AudioFeatures` con 13 campos acústicos (rms, bands, centroid, flux, onset, beat, bpm)
- `BandRatios` con 5 bandas de frecuencia
- Pipeline DSP completo: FFT → bandas → centroid → flux → onset → beat → BPM
- `FeatureBus` para publicar features
- `Track`, `Artist`, `Album`, `Genre`, `Source` con metadata rica
- `history` table con play_count, last_played, recently_played
- `tags` table (actualmente = géneros)
- `track_artists`, `track_genres`, `artist_genres` para relaciones
- `History::record()`, `History::recent()`, `History::stats()`
- `TrackListeningStats` con play_count y recencia

### 🆕 Se diseña (FASE 8–10)
- `TrackAcousticProfile` — vector de features promediado por track
- `UserTasteProfile` — perfil derivado del historial
- `metadata_similarity` — matching por artista, género, álbum, tag, década
- `acoustic_similarity` — distancia coseno entre perfiles acústicos
- `user_affinity` — afinidad ponderada por artista, género, acústico, match directo
- `recency_bonus` — decaimiento exponencial
- `popularity_factor` — log-normalizado
- `negative_penalty` — penalización por skips repetidos
- `RecommendationScore` — puntuación final con componentes desglosados
- `ranker` — pipeline completo de generación de candidatos → scoring → ranking

### ❌ No se implementa
- ML / modelos entrenados
- Cuentas remotas
- Backend/API de recomendaciones
- Sincronización cloud
- Feature engineering inexistente (MFCC, chroma, ZCR, rolloff, etc.)