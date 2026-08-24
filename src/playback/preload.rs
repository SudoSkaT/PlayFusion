//! Preparación anticipada del SIGUIENTE track (spec §36).
//!
//! Reglas duras:
//! - SOLO el siguiente track: nunca se resuelve una playlist entera;
//! - solo cuando el actual está por terminar (`lead_time`);
//! - sin descargas innecesarias: "preparar" aquí significa WARM DE CACHÉ
//!   (`resolver.resolve`) — si ya está cacheada, es un hit barato; si no, se
//!   resuelve una vez y queda lista;
//! - dedupe por identificador: ticks repetidos del reloj no lanzan tareas
//!   duplicadas; las tareas huérfanas son imposibles (una sola tarea viva por
//!   clave, que limpia su registro al terminar).

use std::sync::Arc;
use std::time::Duration;

use crate::domain::track::Track;
use crate::media::StreamResolver;

/// Configuración del preload.
#[derive(Debug, Clone)]
pub struct PreloadConfig {
    pub enabled: bool,
    /// Cuánto antes del final se dispara la preparación del siguiente.
    pub lead_time: Duration,
}

impl Default for PreloadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lead_time: Duration::from_secs(30),
        }
    }
}

/// Decisión PURA: ¿toca preparar el siguiente?
///
/// Sin duración conocida no hay señal fiable de proximidad: no se pre-carga
/// (evita resolver en cuanto arranca una canción con metadata pobre).
pub fn should_preload(
    position: Duration,
    duration: Option<Duration>,
    has_next: bool,
    config: &PreloadConfig,
) -> bool {
    if !config.enabled || !has_next {
        return false;
    }
    match duration {
        Some(total) if total > config.lead_time => {
            total.saturating_sub(position) <= config.lead_time
        }
        _ => false,
    }
}

/// Gestor de preload sobre el resolver compartido.
pub struct PreloadManager {
    resolver: Arc<StreamResolver>,
    config: PreloadConfig,
    /// Identificadores con resolución en vuelo (dedupe).
    inflight: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}

impl std::fmt::Debug for PreloadManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreloadManager")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl PreloadManager {
    pub fn new(resolver: Arc<StreamResolver>, config: PreloadConfig) -> Self {
        Self {
            resolver,
            config,
            inflight: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    pub fn config(&self) -> &PreloadConfig {
        &self.config
    }

    /// Evalúa y, si procede, lanza el warm de caché para `next`.
    ///
    /// Fire-and-forget deliberado: el resultado vive en la caché del resolver,
    /// así que nadie necesita esperar el valor; un fallo de preload NO es un
    /// fallo de reproducción (se resolverá normal al saltar).
    pub async fn consider(&self, next: Option<Track>) {
        if !self.config.enabled {
            return;
        }
        let Some(track) = next else {
            return;
        };
        let key = track.identifier();
        {
            let mut inflight = self.inflight.lock().await;
            if !inflight.insert(key.clone()) {
                return; // ya va en marcha para este track
            }
        }
        tracing::debug!(key = %key, "preload_started");
        let resolver = self.resolver.clone();
        let inflight = self.inflight.clone();
        tokio::spawn(async move {
            match resolver.resolve(&track).await {
                Ok(_) => tracing::debug!(key = %key, "preload_ready"),
                Err(e) => tracing::debug!(key = %key, error = %e, "preload_failed"),
            }
            inflight.lock().await.remove(&key);
        });
    }

    #[cfg(test)]
    pub async fn is_inflight(&self, key: &str) -> bool {
        self.inflight.lock().await.contains(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source::Source;
    use crate::media::cache::MemoryResolutionCache;
    use crate::media::registry::StreamRegistry;
    use crate::media::resolver::{ResolverConfig, StreamResolver};
    use crate::media::test_support::{FakeStreamProvider, Step};

    fn track(id: &str) -> Track {
        let mut t = Track::new(id.to_string(), Vec::new(), Source::YouTube);
        t.external_id = Some(id.to_string());
        t
    }

    #[test]
    fn decision_matrix_matches_spec() {
        let cfg = PreloadConfig::default();
        let long = Duration::from_secs(200);

        assert!(!should_preload(Duration::ZERO, Some(long), true, &cfg), "lejos del final");
        assert!(
            should_preload(long - Duration::from_secs(30), Some(long), true, &cfg),
            "en la ventana del lead time"
        );
        assert!(
            should_preload(long - Duration::from_secs(5), Some(long), true, &cfg),
            "dentro del último tramo"
        );
        assert!(!should_preload(Duration::ZERO, Some(long), false, &cfg), "sin siguiente");
        assert!(!should_preload(Duration::ZERO, None, true, &cfg), "sin duración");
        assert!(
            !should_preload(Duration::ZERO, Some(long), true, &PreloadConfig { enabled: false, ..Default::default() }),
            "apagado"
        );
        // Pista más corta que el lead time: siempre estaría "cerca del final",
        // pero no hay margen útil → no pre-cargar.
        assert!(!should_preload(Duration::ZERO, Some(Duration::from_secs(10)), true, &cfg));
    }

    #[tokio::test(start_paused = true)]
    async fn dedupes_concurrent_considerations_for_same_track() {
        let provider = Arc::new(FakeStreamProvider::new(
            "a",
            Source::YouTube,
            100,
            vec![Step::SlowOk(Duration::from_millis(500))],
        ));
        let reg = Arc::new(StreamRegistry::new());
        reg.register(provider.clone());
        let resolver = Arc::new(StreamResolver::new(
            reg,
            Arc::new(MemoryResolutionCache::default()),
            ResolverConfig::default(),
        ));
        let manager = PreloadManager::new(resolver, PreloadConfig::default());

        manager.consider(Some(track("v1"))).await;
        // Cede para que la tarea spawneada arranque y llegue a su sleep
        // (donde ya incrementó el contador de llamadas).
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        manager.consider(Some(track("v1"))).await; // tick repetido
        manager.consider(Some(track("v1"))).await;

        assert!(
            manager.is_inflight("v1").await,
            "la primera tarea sigue en vuelo (clock pausado)"
        );
        assert_eq!(provider.call_count(), 1, "los duplicados no lanzan tareas");

        // Al terminar, la tarea limpia su registro.
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert!(!manager.is_inflight("v1").await);
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn disabled_manager_does_not_spawn_anything() {
        let provider = Arc::new(FakeStreamProvider::ok("a", Source::YouTube, "uri"));
        let reg = Arc::new(StreamRegistry::new());
        reg.register(provider.clone());
        let resolver = Arc::new(StreamResolver::new(
            reg,
            Arc::new(MemoryResolutionCache::default()),
            ResolverConfig::default(),
        ));
        let manager = PreloadManager::new(
            resolver,
            PreloadConfig {
                enabled: false,
                ..Default::default()
            },
        );

        manager.consider(Some(track("v1"))).await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert_eq!(provider.call_count(), 0);
    }
}
