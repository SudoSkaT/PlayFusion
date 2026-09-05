# PlayFusion

Reproductor de música en TUI para YouTube (sin API key): busca canciones en
YouTube Music, resuelve streams de audio con cliente Android/iOS (PO tokens),
reproduce con rodio/symphonia y sincroniza letras (LRCLIB) con karaoke.

## Configuración

La configuración se lee de variables de entorno (o un `.env` en la raíz):

| Variable            | Descripción                                                       |
| ------------------- | ----------------------------------------------------------------- |
| `PLAYBACK_POLICY`   | `auto` (por fuente) o `rodio` (forzar el motor local)             |
| `HTTP_PROXY`        | Proxy para todas las peticiones a YouTube (ver sección Proxy)     |
| `HTTPS_PROXY`       | Igual que `HTTP_PROXY` para conexiones TLS                        |
| `ALL_PROXY`         | Igual que `HTTP_PROXY` para cualquier protocolo                   |
| `NO_PROXY`          | Lista de hosts que se conectan sin proxy                          |

## Límites de streaming (diagnóstico 2026-08)

El síntoma «la reproducción se corta alrededor de 1 MiB (~65 s)» fue
diagnosticado con evidencia HTTP (probes en `examples/probe_*.rs`):

- googlevideo exige `Range` cerrado: un GET completo responde **403**.
- Las URLs resueltas con clientes directos capados (ANDROID_VR/iOS) tienen un
  **techo posicional por URL** (~1,02–1,07 MiB): cualquier rango cuyo fin lo
  supere responde **403**; las repeticiones dentro del techo se sirven
  al instante a plena velocidad. **No es cuota por IP** ni expiración.
- Las URLs del cliente **VISIONOS** (usado como primario desde el parche del
  vendored rustypipe) no tienen ese techo y sirven el archivo completo.
- El provider verifica cada stream con una sonda más allá de 1 MiB antes de
  aceptarlo (`stream_url_ok`); si VISIONOS fallara, Android/iOS quedan de
  respaldo (con prefijo servible + aviso honesto `Cut`, nunca autoplay ciego).
- La descarga usa ventanas Range encadenadas con validación estricta
  ([`HttpRangeStream`], capa Media): retries acotados solo para fallos
  transitorios; 403/416/200-forzado/Content-Range inválido se clasifican sin
  reintentos ciegos.

Herramientas de diagnóstico:

```sh
cargo run --release --example probe_range      # GET vs rangos A–F sobre 1 resolución
cargo run --release --example probe_boundary   # frontera exacta del techo
cargo run --release --example probe_frontier   # ¿el techo avanza? (no)
cargo run --release --example probe_clients    # techo por contexto de cliente
cargo run --release --example probe_seek       # E2E: pista COMPLETA hasta Finished
```

El proxy del entorno sigue respetándose (`PROXY_ENABLED`), pero ya no es parte
de la solución: la causa nunca fue la IP.

## Visualizador de audio

Al reproducir una canción aparece una banda **Visual** entre la tarjeta del
disco y la barra de progreso: espectro de barras reactivo (graves a la
izquierda), punto de pulso ● en el título que late con el beat y color según
la intensidad de agudos. Debajo, una capa ambiental de **lava** (metaballs)
reacciona a energía/brillo/distorsión del PCM, y su **paleta se deriva de la
portada** de la canción. La fase deriva SIEMPRE de la posición real de
reproducción (sin relojes visuales propios), y la transición al hacer seek se
suaviza en vez de tele-transportar. En la vista *Related*, la tecla **`v`**
cicla entre Auto / Letras / Visual, y las letras karaoke se superponen sobre la
lava aplacada sin borrarle el fondo.

Requiere salida de audio real funcionando (rodio/cpal). Se controla desde
`.env` sin recompilar:

| Variable                        | Default | Descripción                                  |
| ------------------------------- | ------- | -------------------------------------------- |
| `AUDIO_ANALYSIS_ENABLED`        | `1`     | Análisis PCM→features en hilo dedicado       |
| `ADVANCED_VISUALIZATION_ENABLED`| `1`     | Renderizado del visualizador en la TUI       |

Si el análisis está apagado, la banda se muestra gris e inactiva.

## Rendimiento (medido, spec §34)

Banco de caminos calientes: `cargo run --release --example bench_hotpaths`.

| Métrica | Valor | Presupuesto | Margen |
| --- | --- | --- | --- |
| Frame de análisis completo (FFT 2048 + bandas + flux) | ~15 µs | 11.6 ms/hop | ×780 |
| Render visual TUI (80×5) | ~32 µs | 66 ms/tick | ×2000 |
| Anillo SPSC audio→análisis | ~630 M muestras/s | 96 k/s | ×6500 |
| Hilo de análisis end-to-end | ~23× tiempo real | 1× | ✓ |

El camino caliente del DSP y el render no hacen allocations por frame
(buffers reutilizados; el único alloc por frame es el snapshot `Arc` que se
publica al bus). El análisis corre en hilo propio y jamás bloquea el audio:
si se atrasa, descarta muestras nuevas (drop-newest) en vez de cortar la
reproducción.