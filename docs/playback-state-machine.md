# FASE 3 — Máquina de Estados de Playback

## Estados del modelo

De los nueve estados candidatos, **cinco son necesarios** para PlayFusion. Los otros cuatro se descartan o se fusionan con estados existentes:

| Estado | ¿Necesario? | Justificación |
|--------|-------------|---------------|
| `Stopped` | **Sí** | Fin natural, parada explícita o error terminal. Es el estado base. Ya existe. |
| `Loading` | **Sí** | La canción está en proceso de resolución: descarga de metadatos, duración, bitrate. Antes de que el stream esté listo para buffer. **Nuevo.** |
| `Buffering` | **Sí** | Se tiene el stream pero no hay suficientes datos descargados para reproducir continuamente. Ya existe. |
| `Playing` | **Sí** | La reproducción activa consume datos y suena. Ya existe. |
| `Paused` | **Sí** | El usuario detuvo la reproducción manualmente. Ya existe. |
| `Seeking` | **Sí** | Seek en curso; el audio puede sonar desde la posición actual mientras se descarga la región objetivo. Ya se añadió. |
| `Recovering` | **Sí** | Error de transporte detectado; se intenta re-resolver el stream (una vez por track). **Nuevo.** |
| `Finished` | **Sí** | El stream llegó a EOF limpio y el player ya no tiene fuentes. **Nuevo.** |
| `Error` | **No** | Se fusiona con `Stopped` + `PlaybackEvent::Error`. El estado `Error` duplica la información de `Stopped` cuando ya hay un evento de error. |
| `Preloaded` | **No** | Es un estado de *preparación del siguiente track*, no de la reproducción actual. Se elimina como variante de `PlaybackState`; la pre-carga se gestiona en `PreloadManager` sin exponer estado. |

## Máquina de estados resultante

```
                    ┌──────────────────────────────────────────────┐
                    │                                              │
                    ▼                                              │
              ┌─────────┐   Play(track)   ┌──────────┐              │
    ┌───────►│ Stopped │────────────────►│ Loading  │              │
    │        └─────────┘                 └────┬─────┘              │
    │                                        │                    │
    │                              TrackReady│                    │
    │                                        ▼                    │
    │                               ┌─────────────┐              │
    │                               │ Buffering   │              │
    │                               └──────┬──────┘              │
    │                                      │                     │
    │                        BufferReady   │    Error            │
    │                                      ▼                     │
    │                               ┌───────────┐               │
    │                               │ Playing   │               │
    │                               └──┬───┬──┬─┘               │
    │                                  │   │  │                  │
    │                         Pause|   |  |  |Seek               │
    │                                  │   │  |                  │
    │                                  ▼   ▼  ▼                  │
    │                            ┌──┐ ┌──┐ ┌───────┐            │
    │                            │Paused│ │Seeking│            │
    │                            └──┘ └──┘ └───┬───┘            │
    │                                         │                │
    │                                  SeekCompleted           │
    │                                         │                │
    │                                         ▼                │
    │                               ┌─────────────┐            │
    │                               │ Buffering   │            │
    │                               └─────────────┘            │
    │                                                            │
    │                            Error (transport)               │
    │                                  │                         │
    │                                  ▼                         │
    │                          ┌──────────────┐                 │
    │                          │  Recovering  │                 │
    │                          └──────┬───────┘                 │
    │                                 │                         │
    │                    RecoverySucceeded  RecoveryFailed       │
    │                                 │                         │
    │                                 ▼                         │
    │                          ┌────────────┐    ┌──────────┐   │
    │                          │Buffering   │    │  Stopped │   │
    │                          └────────────┘    └──────────┘   │
    │                                                            │
    │                   EOF + player.empty()                      │
    │                                  │                         │
    │                                  ▼                         │
    │                          ┌────────────┐                    │
    │                          │ Finished   │────────────────────┘
    │                          └────────────┘
    └──────────────────────────────────────────────────────────────
```

## Comandos y transiciones

### Comandos

| Comando | Descripción |
|---------|-------------|
| `Play(track)` | Inicia o reanuda la reproducción de `track` |
| `Pause` | Pausa la reproducción en curso |
| `Resume` | Reanuda tras `Paused` |
| `Seek(position)` | Solicita un salto a `position` |
| `Stop` | Detiene la reproducción y vuelve a `Stopped` |
| `Next` | Avanza al siguiente track de la cola |
| `Previous` | Retrocede al track anterior |
| `Recover` | Re-intenta tras un fallo de transporte |

### Tabla de transiciones

| Estado actual | Comando/Evento | Siguiente estado | Evento emitido | Efecto lateral |
|---------------|----------------|------------------|----------------|----------------|
| `Stopped` | `Play(track)` | `Loading` | `TrackLoading(track)` | Se establece `track` actual; se genera nueva session |
| `Loading` | `TrackReady(metadata)` | `Buffering` | `TrackReady(track)` | Se tiene la URI del stream; se inicia descarga |
| `Loading` | `PlaybackError` | `Stopped` | `PlaybackError(err)` | Se limpia el track actual |
| `Buffering` | `BufferReady` | `Playing` | `PlaybackStarted` | Se append al player; se inicia monitor |
| `Buffering` | `PlaybackError` | `Recovering` | `RecoveryStarted(err)` | Se arma RecoveryBudget |
| `Playing` | `Pause` | `Paused` | `PlaybackPaused` | `player.pause()` |
| `Playing` | `Seek(pos)` | `Seeking` | `SeekStarted(from, to)` | Se genera nueva session; se flush del análisis |
| `Playing` | `Stop` | `Stopped` | `PlaybackStopped` | Se cancela el buffer; se limpia el track |
| `Playing` | `EOF + empty` | `Finished` | `TrackFinished` | Se emite `PlaybackEvent::Finished` |
| `Playing` | `TransportError` | `Recovering` | `RecoveryStarted(err)` | Se invalida la resolución |
| `Paused` | `Resume` | `Playing` | `PlaybackResumed` | `player.play()` |
| `Paused` | `Stop` | `Stopped` | `PlaybackStopped` | Se cancela todo |
| `Paused` | `Seek(pos)` | `Seeking` | `SeekStarted(from, to)` | Se genera nueva session |
| `Seeking` | `SeekCompleted` | `Buffering` | `SeekCompleted(position)` | Se re-ancla PositionClock; se reanuda monitor |
| `Seeking` | `Stop` | `Stopped` | `PlaybackStopped` | Se cancela el seek |
| `Recovering` | `RecoverySucceeded` | `Buffering` | `RecoverySucceeded` | Se re-resuelve el stream; se reinicia buffer |
| `Recovering` | `RecoveryFailed` | `Stopped` | `RecoveryFailed(err)` | Se notifica al usuario; se limpia |
| `Finished` | `Play(track)` | `Loading` | `TrackLoading(track)` | Replay del mismo o nuevo track |
| `Finished` | `Stop` | `Stopped` | `PlaybackStopped` | Ya estaba terminada |

### Estado `Loading` — justificación

`Loading` separa la fase de **resolución** (descubrir la URL del stream, obtener metadatos, duración) de la fase de **buffer** (descargar datos suficientes para empezar). Esto permite:

- Mostrar un estado explícito al usuario: "cargando metadatos" vs "preparando audio"
- El `PlaybackEngine::play()` ya hace esto internamente: abre el transporte, obtiene la duración, bufferiza. Exponer `Loading` como estado refleja esa realidad
- Si la resolución falla (red caída, URL rechazada), se puede emitir `PlaybackError` sin llegar nunca a `Buffering`

### Estado `Recovering` — justificación

`Recovering` gestiona el caso donde el stream falla durante la reproducción (corte de red, restricción temporal del CDN). La recuperación es:

- **Acotada**: un solo reintento por track (`RecoveryBudget`)
- **Asíncrona**: se re-resuelve el stream sin bloquear el hilo de audio
- **Observable**: el UI muestra "reconectando…" mientras se espera

### Estado `Finished` — justificación

Separar `Finished` de `Stopped` es importante porque:

- El `autoplay` solo debe saltar a la siguiente canción tras `Finished` natural (EOF + buffer vacío), no tras `Stopped` explícito
- El karaoke limpia el panel solo al `Finished` real, no al `Stopped`
- El monitor de buffering sabe que no debe rellenar si la canción terminó

---

## FASE 4 — Concurrencia y operaciones obsoletas

### Garantía arquitectónica

> **Ninguna operación asíncrona perteneciente a una reproducción anterior puede modificar el estado de una reproducción posterior.**

### Mecanismo actual de `generation`

El `RodioBackend` ya usa `generation: Arc<AtomicU64>` para invalidar tareas obsoletas. Cada `play()`/`stop()` incrementa la generación. Las tareas de descarga y monitor capturan su valor y salen si el valor cambia.

**Problema**: este mecanismo solo cubre el `RodioBackend`. Otras operaciones asíncronas no están protegidas de la misma forma.

### Cobertura actual y brechas

| Operación | Protegida por generation? | Riesgo |
|-----------|---------------------------|--------|
| Descarga de stream (tokio::spawn en `play()`) | **Sí** — verifica `generation` | Bajo |
| Monitor de buffering (tokio::spawn en `spawn_monitor()`) | **Sí** — verifica `generation` | Bajo |
| Prefetch de seek region (`prefetch_seek_region`) | **Parcial** — verifica `buffer.failed()` pero no generation | Medio |
| Preload (`PreloadManager.consider`) | **No** — no hay generación | Bajo (fire-and-forget, no modifica estado de playback) |
| Descarga de thumbnails | **No** — pero usa `key` como identificador | Medio |
| Descarga de letras (LRCLIB) | **No** — necesita identificador de track | **Alto** |
| Recomendaciones (`LoadRelated`) | **No** — necesita `now_playing` como ancla | **Alto** |
| Análisis DSP (hilo dedicado) | **Sí** — el `TapSource` se adjunta a `AnalysisRuntime` nuevo | Bajo |

### Patrón de solución: `session_id` explícito

Cada operación asíncrona que modifique estado de playback debe llevar un `session_id` que coincida con el de la reproducción actual. El mecanismo:

```
// En el backend / aplicación:
struct PlaybackSession {
    id: u64,          // generación actual
    track_key: String, // track actual
    started_at: Instant,
}

// Cada operación asíncrona lleva su session_id:
async fn fetch_lyrics(track: Track, session_id: u64) -> Result<SyncLyrics, ()> {
    let lyrics = lrclib::fetch(&track).await?;
    // Verificar que la sesión sigue siendo la vigente
    if current_session_id() != session_id {
        return Err(StaleOperation); // DESCARTAR
    }
    Ok(lyrics)
}
```

### Race conditions analizadas

#### 1. A → B → A (cambio rápido de track)

```
Track A (session = 10) → Track B (session = 11) → Track A (session = 12)
```

- Resultado tardío de A (session=10) → session actual=12 → **DESCARTAR**
- Resultado tardío de B (session=11) → session actual=12 → **DESCARTAR**
- Resultado de A (session=12) → session actual=12 → **ACEPTAR**

**Solución**: El `session_id` es un contador monotónico. Cada operación asíncrona captura el `session_id` al iniciarse. Al retornar, verifica que coincida.

#### 2. A → seek → B (seek durante cambio de track)

```
Playing A → Seek(50s) → Play B
```

- El seek de A puede tardar en confirmarse. Al cambiar a B, `PositionClock::clear()` se llama.
- El `PlaybackEngine` incrementa la generación → el seek pendiente de A se invalida.
- `PositionClock::update(key_B, ...)` detecta track nuevo → emite `ClockEvent::NewTrack` → se limpian las letras.

**Solución**: `PositionClock::update()` con `key_B` detecta track nuevo y limpia el estado anterior. El `seek` pendiente de A se pierde al cambiar de `track_key`.

#### 3. A → stop → B (parada durante cambio de track)

```
Playing A → Stop → Play B
```

- `stop()` incrementa la generación → cualquier tarea de A sale.
- `Play(B)` genera nueva sesión → B arranca limpio.

**Solución**: El `generation` del `RodioBackend` cubre esto. Se extiende a `session_id` para las operaciones de red.

#### 4. A → recovery → B (recuperación durante cambio de track)

```
Playing A (transport error) → Recovering → Play B
```

- La recuperación de A intenta re-resolver el stream de A.
- Al cambiar a B, la generación cambia → el `RecoveryBudget` se rearma para B.
- La resolución de A (si llega tarde) verifica session → **DESCARTAR**.

**Solución**: `RecoveryBudget::arm(key)` se llama al iniciar B → cualquier reintento para A con key=A no pasa `try_consume(A)`.

#### 5. A → preload → B (preload del siguiente track)

```
Playing A → Preload(B) → Play B
```

- `PreloadManager` resuelve B en background.
- Al cambiar a B, el resultado del preload de B es válido (mismo track).
- El resultado del preload de C (si se lanzó durante A→B) es obsoleto.

**Solución**: El `PreloadManager` usa `inflight: HashSet<String>` con dedup por identificador. Si B ya está en caché, el preload es un hit barato. No hay problema de obsolescencia porque el preload no modifica estado de playback.

### Extensión del `session_id` a todas las operaciones

```rust
/// Identificador de sesión de reproducción. Cada play()/stop()
/// incrementa el contador; cualquier operación asíncrona que
/// pertenezca a una sesión anterior es descartada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionId(u64);

/// Estado de una reproducción protegido por session_id.
pub struct PlaybackSession {
    pub id: SessionId,
    pub track_key: Option<String>,
}

impl PlaybackSession {
    /// Genera una nueva sesión (nuevo play o replay).
    pub fn next(&mut self) -> SessionId {
        self.id.0 += 1;
        self.id
    }

    /// Verifica si una operación obsoleta.
    pub fn is_stale(&self, op_session: SessionId) -> bool {
        op_session != self.id
    }
}
```

### Operaciones que necesitan `session_id`

| Operación | Dónde se valida | Acción si stale |
|-----------|-----------------|-----------------|
| Descarga de stream | `RodioBackend::play()` | La tarea sale por `generation` |
| Monitor de buffering | `spawn_monitor()` | La tarea sale por `generation` |
| Re-resolución en `Recovering` | `decide_recovery()` → `RecoveryBudget` | `try_consume()` falla |
| Descarga de letras | `fetch_lyrics()` | Verifica `session_id` al retornar; si stale, ignora |
| Descarga de thumbnails | `load_thumbnail(track)` | Verifica `now_playing` al retornar; si cambió, ignora |
| Recomendaciones | `load_related(track)` | Verifica `now_playing` al retornar; si cambió, ignora |

---

## FASE 6 — Karaoke

### Principio fundamental

> **El karaoke NO debe adivinar independientemente la posición. Solo debe depender de `PositionClock`.**

### Flujo de sincronización

```
Track A
  ↓
Lyrics A (SyncLyrics, parseadas de LRCLIB)
  ↓
PositionClock.update(key_A, position)
  ↓
ClockEvent::NewTrack → clear_lyrics() → descarta Lyrics A
  ↓
PositionClock.tick() → snapshot() → position
  ↓
active_index = SyncLyrics::active_index(position)
  ↓
KaraokeScroller.advance(lyrics, active, finished, height)
  ↓
Render: línea activa resaltada
```

### Garantías de sincronización

1. **Después de `seek`**: `PositionClock::update()` con la nueva posición reportada → `snapshot()` refleja la posición correcta → el karaoke se desplaza a la línea correcta.

2. **Después de `pause`**: `PositionClock::snapshot(playing=false, ...)` devuelve `position` congelada → el karaoke se congela en la línea actual.

3. **Después de `buffering`**: `PositionClock::snapshot(playing=true, stalled=true, ...)` devuelve `position` congelada → el karaoke no avanza.

4. **Después de `recovery`**: `PositionClock` se re-ancla → el karaoke sigue la nueva posición.

5. **Después de cambio de track**: `ClockEvent::NewTrack` → `clear_lyrics()` → `scroll.reset()` → `synced = None` → la letra de la canción anterior se descarta completamente.

6. **Lyrics obsoletas**: Si `fetch_lyrics(track_A)` llega después de que el usuario escuchó `track_B`, `session_id` de la petición no coincide → **se ignora**. `related.set_synced()` no se llama.

### Diagrama de sequence: seek + karaoke

```
Usuario          Backend          PositionClock       Karaoke
  │                 │                    │                    │
  │── Seek(50s) ──►│                    │                    │
  │                 │── begin_seek(50s) ──►│                    │
  │                 │                    │                    │
  │                 │── seek_complete ────►│── update(key, 50) ──►│
  │                 │                    │                    │── advance(lyrics, active=5, ...)
  │                 │                    │◄── NewTrack (si cambia)
  │                 │                    │                    │── clear_lyrics()
  │◄── SeekComplete ──│                    │                    │
```

### Diagrama de sequence: cambio de track + lyrics obsoletas

```
UI                  Backend          LRCLIB           PositionClock
  │                    │                  │                    │
  │── Play(A) ──────►│                  │                    │
  │                    │── fetch_lyrics(A) ──►│                  │
  │                    │                  │  (lento)            │
  │── Play(B) ──────►│                  │                    │
  │                    │── session = B    │                    │
  │                    │── clear_lyrics() ──►│                  │
  │                    │◄── fetch_lyrics(A) ──│  (llega tarde)    │
  │                    │                    │                    │
  │                    │   session_id(A) != session(B)       │
  │                    │   → DESCARTAR lyrics de A           │
```

### Tests requeridos

| Test | Descripción |
|------|-------------|
| `karaoke_normal_playback` | Posición avanza → línea activa avanza → ventana se desliza |
| `karaoke_pause` | Al pausar, `snapshot` congela → línea activa no cambia |
| `karaoke_resume` | Al reanudar, `snapshot` extrapola → línea activa avanza |
| `karaoke_seek_forward` | Seek adelante → PositionClock re-ancla → línea activa salta |
| `karaoke_seek_backward` | Seek atrás → PositionClock re-ancla → línea activa retrocede |
| `karaoke_buffering` | En buffering (stalled=true) → posición congelada → karaoke no avanza |
| `karaoke_track_change` | Cambio de track → ClockEvent::NewTrack → clear_lyrics() |
| `karaoke_stale_lyric_response` | Lyrics de track anterior llegan tarde → se ignoran |
| `karaoke_reset_on_clear` | clear() → PositionClock se resetea → karaoke se limpia |
| `karaoke_timestamp_discontinuity` | Timestamps discontinuos (seek) → active_index saltos correctos |
| `karaoke_finished` | Fin real de la canción → `finished=true` → karaoke se limpia |

---

## Implementación planificada

### FASE 3 — Cambios en el código

1. **`src/app/audio.rs`**: Añadir `Loading`, `Finished` a `PlaybackState`; eliminar `Preloaded`; añadir `Recovering`.
2. **`src/ui/app.rs`**: Actualizar `animation_active()` y `playback_line()`.
3. **`src/ui/dashboard/mod.rs`**: Actualizar matches de estado.
4. **`src/infrastructure/playback/rodio_backend.rs`**: Añadir lógica de estado `Loading` → `Buffering` → `Playing`; `Finished` en `status()`.

### FASE 4 — Cambios en el código

1. **`src/app/audio.rs`**: Añadir `SessionId` y `PlaybackSession`.
2. **`src/infrastructure/playback/rodio_backend.rs`**: Extender `generation` al `prefetch_seek_region`.
3. **`src/providers/youtube/lyrics.rs`**: Pasar `session_id` a `fetch_lyrics`.
4. **`src/ui/app.rs`**: Validar `session_id` al recibir lyrics/recomendaciones.

### FASE 6 — Tests en el código

1. **`src/playback/karaoke_tests.rs`**: Módulo de tests del karaoke con `PositionClock` + `KaraokeScroller` + `SyncLyrics`.
2. **Tests de lyrics stale**: Verificar que lyrics obsoletas no modifican el estado.
