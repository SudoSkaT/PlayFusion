//! Gestor de expiración: UN solo lugar que razona sobre vigencia de
//! resoluciones (spec §14).
//!
//! Responsabilidades:
//! - **detectar** entradas caducadas y barrerlas de la caché;
//! - **detectar próximas a caducar** dentro de una ventana (refresh preventivo);
//! - entregar esos hallazgos al coordinador (Fase 3: preload/recovery del
//!   motor), que decide QUÉ refrescar — solo el track en curso o el siguiente,
//!   jamás toda una playlist (spec §36).
//!
//! El manager NO refresca por su cuenta: no conoce el coste ni el contexto de
//! reproducción. La lógica de expiración NO se duplica en otras clases: la
//! caché filtra caducadas al leer; este módulo las barre y anticipa.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::media::cache::ResolutionCache;

/// Gestor de vigencia sobre un puerto de caché.
pub struct ExpirationManager {
    cache: Arc<dyn ResolutionCache>,
    /// Ventana de "próximo a caducar" para refresh preventivo.
    near_window: Duration,
}

/// Resultado de un barrido.
#[derive(Debug, Default, Clone)]
pub struct SweepReport {
    /// Claves eliminadas por estar ya caducadas.
    pub purged: Vec<String>,
    /// Claves vivas que caducan dentro de la ventana preventiva, con su
    /// instante de vencimiento.
    pub expiring_soon: Vec<(String, DateTime<Utc>)>,
}

impl ExpirationManager {
    pub fn new(cache: Arc<dyn ResolutionCache>, near_window: Duration) -> Self {
        Self { cache, near_window }
    }

    /// Ventana preventiva configurada.
    pub fn near_window(&self) -> Duration {
        self.near_window
    }

    /// Barre la caché: elimina caducadas y devuelve el informe completo
    /// (purgadas + próximas a caducar).
    ///
    /// Pensado para llamadas periódicas del coordinador de playback o justo
    /// antes de decidir un preload.
    pub async fn sweep(&self) -> SweepReport {
        let expired = self.cache.expiring_within(Duration::ZERO).await;
        for (key, _) in &expired {
            self.cache.invalidate(key).await;
        }

        let expiring_soon = if self.near_window.is_zero() {
            Vec::new()
        } else {
            self.cache.expiring_within(self.near_window).await
        };

        SweepReport {
            purged: expired.into_iter().map(|(k, _)| k).collect(),
            expiring_soon,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source::Source;
    use crate::domain::stream::StreamResolution;
    use crate::media::cache::MemoryResolutionCache;

    fn resolution(uri: &str, expires_at: Option<DateTime<Utc>>) -> StreamResolution {
        StreamResolution {
            expires_at,
            ..StreamResolution::new(Source::YouTube, uri)
        }
    }

    #[tokio::test]
    async fn sweep_purges_expired_and_reports_near_expiry() {
        let cache = Arc::new(MemoryResolutionCache::default());
        let now = Utc::now();

        // Caducada hace 1 h → purgada.
        let mut dead = resolution("https://cdn/dead", None);
        dead.expires_at = Some(now - chrono::Duration::hours(1));
        // Caduca en 2 min → reportada como próxima.
        let mut soon = resolution("https://cdn/soon", None);
        soon.expires_at = Some(now + chrono::Duration::minutes(2));
        // Sin vencimiento → intocable.
        let immortal = resolution("https://cdn/immortal", None);

        cache.put("dead", dead).await;
        cache.put("soon", soon).await;
        cache.put("immortal", immortal).await;

        let manager = ExpirationManager::new(cache.clone(), Duration::from_secs(5 * 60));
        let report = manager.sweep().await;

        assert_eq!(report.purged, vec!["dead".to_string()]);
        assert_eq!(report.expiring_soon.len(), 1);
        assert_eq!(report.expiring_soon[0].0, "soon");

        // La caducada desapareció; las demás siguen.
        assert!(cache.get("dead").await.is_none());
        assert!(cache.get("soon").await.is_some());
        assert!(cache.get("immortal").await.is_some());
    }

    #[tokio::test]
    async fn zero_window_disables_preventive_reporting() {
        let cache = Arc::new(MemoryResolutionCache::default());
        let mut soon = resolution("https://cdn/soon", None);
        soon.expires_at = Some(Utc::now() + chrono::Duration::minutes(1));
        cache.put("soon", soon).await;

        let manager = ExpirationManager::new(cache, Duration::ZERO);
        let report = manager.sweep().await;
        assert!(report.purged.is_empty(), "no está caducada");
        assert!(
            report.expiring_soon.is_empty(),
            "ventana cero: sin informes preventivos"
        );
    }
}
