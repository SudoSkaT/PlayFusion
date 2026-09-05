//! Caché de resoluciones de stream.
//!
//! Separa CONCEPTUALMENTE la caché de resolución (URIs potencialmente
//! temporales) de cualquier otra caché del sistema: sus entradas viven y
//! mueren por `expires_at`, nunca "para siempre".
//!
//! - [`ResolutionCache`]: puerto (trait) que consume el resolver.
//! - [`MemoryResolutionCache`]: capa caliente acotada en memoria.
//! - [`TwoTierCache`]: compuesto caliente+frió (p. ej. SQLite) con lectura en
//!   cascada y escritura a ambos niveles.
//!
//! La implementación persistente (SQLite) vive en Infraestructura e implementa
//! este mismo puerto: el resolver nunca sabe dónde duermen las entradas.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::{source::Source, stream::StreamResolution};

/// Puerto de caché de resoluciones.
///
/// Las claves son identificadores estables de track
/// (`Track::identifier()`). Toda operación es asíncrona para permitir
/// backends remotos sin cambiar el contrato.
#[async_trait]
pub trait ResolutionCache: Send + Sync {
    /// Entrada vigente para `key`. Una entrada caducada se considera inexistente
    /// (y el backend puede purgarla).
    async fn get(&self, key: &str) -> Option<StreamResolution>;

    /// Guarda/actualiza la resolución de `key` respetando su `expires_at`.
    async fn put(&self, key: &str, resolution: StreamResolution);

    /// Elimina la entrada de `key`.
    async fn invalidate(&self, key: &str);

    /// Elimina TODAS las entradas de un origen (provider caído, flag off...).
    async fn clear_provider(&self, source: Source);

    /// Pares (clave, expiración) que caducan dentro de `window` desde ahora,
    /// para decisiones de refresh preventivo ([`crate::media::ExpirationManager`]).
    ///
    /// Un backend sin noción de expiración fina devuelve vacío: su poda es
    /// perezosa (por edad al leer).
    async fn expiring_within(&self, window: Duration) -> Vec<(String, DateTime<Utc>)>;
}

/// Capa caliente en memoria, acotada.
///
/// - `get` filtra expiradas (y las purga al vuelo).
/// - Evicción FIFO por inserción cuando se supera `capacity`: una caché de
///   resoluciones no debe crecer con el historial de reproducción.
#[derive(Debug)]
pub struct MemoryResolutionCache {
    entries: tokio::sync::Mutex<MemInner>,
    capacity: usize,
}

#[derive(Debug, Default)]
struct MemInner {
    map: HashMap<String, StreamResolution>,
    /// Orden de inserción para evicción FIFO.
    order: VecDeque<String>,
}

impl MemoryResolutionCache {
    /// Caché con capacidad máxima de `capacity` entradas vivas.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: tokio::sync::Mutex::new(MemInner::default()),
            capacity: capacity.max(1),
        }
    }
}

impl Default for MemoryResolutionCache {
    fn default() -> Self {
        // 512 tracks es holgado para una sesión y ocupa poco (solo metadata).
        Self::new(512)
    }
}

#[async_trait]
impl ResolutionCache for MemoryResolutionCache {
    async fn get(&self, key: &str) -> Option<StreamResolution> {
        let mut g = self.entries.lock().await;
        let now = Utc::now();
        match g.map.get(key) {
            Some(r) if !r.is_expired_at(now) => Some(r.clone()),
            Some(_) => {
                // Expirada: purga perezosa.
                g.map.remove(key);
                g.order.retain(|k| k != key);
                None
            }
            None => None,
        }
    }

    async fn put(&self, key: &str, resolution: StreamResolution) {
        let mut g = self.entries.lock().await;
        if !g.order.iter().any(|k| k == key) {
            g.order.push_back(key.to_string());
        }
        g.map.insert(key.to_string(), resolution);
        while g.map.len() > self.capacity {
            let Some(oldest) = g.order.pop_front() else {
                break;
            };
            g.map.remove(&oldest);
        }
    }

    async fn invalidate(&self, key: &str) {
        let mut g = self.entries.lock().await;
        g.map.remove(key);
        g.order.retain(|k| k != key);
    }

    async fn clear_provider(&self, source: Source) {
        let mut g = self.entries.lock().await;
        g.map.retain(|_, r| r.provider != source);
        let alive: std::collections::HashSet<String> = g.map.keys().cloned().collect();
        g.order.retain(|k| alive.contains(k));
    }

    async fn expiring_within(&self, window: Duration) -> Vec<(String, DateTime<Utc>)> {
        let g = self.entries.lock().await;
        let deadline = Utc::now() + chrono_duration(window);
        g.map
            .iter()
            .filter_map(|(k, r)| {
                r.expires_at
                    .filter(|exp| *exp <= deadline)
                    .map(|exp| (k.clone(), exp))
            })
            .collect()
    }
}

/// Convierte un `std::time::Duration` a `chrono::Duration` (pérdida cero para
/// los rangos usados aquí).
fn chrono_duration(d: Duration) -> chrono::Duration {
    chrono::Duration::from_std(d).unwrap_or(chrono::Duration::MAX)
}

/// Compuesto caliente+frió: lectura en cascada, escritura a ambos niveles.
///
/// El caso común (replay de una canción reciente) se sirve de la capa
/// caliente; tras un reinicio del proceso la fría evita re-resolver. El
/// promocionado explícito no existe: el resolver valida cada hit y vuelve a
/// `put` si procede.
pub struct TwoTierCache {
    hot: Arc<dyn ResolutionCache>,
    cold: Arc<dyn ResolutionCache>,
    hits_hot: AtomicUsize,
    hits_cold: AtomicUsize,
}

impl std::fmt::Debug for TwoTierCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwoTierCache")
            .field("hits_hot", &self.hits_hot.load(Ordering::Relaxed))
            .field("hits_cold", &self.hits_cold.load(Ordering::Relaxed))
            .finish()
    }
}

use std::sync::Arc;

impl TwoTierCache {
    pub fn new(hot: Arc<dyn ResolutionCache>, cold: Arc<dyn ResolutionCache>) -> Self {
        Self {
            hot,
            cold,
            hits_hot: AtomicUsize::new(0),
            hits_cold: AtomicUsize::new(0),
        }
    }

    /// Métricas de acierto por nivel (observabilidad barata).
    pub fn hit_stats(&self) -> (usize, usize) {
        (
            self.hits_hot.load(Ordering::Relaxed),
            self.hits_cold.load(Ordering::Relaxed),
        )
    }
}

#[async_trait]
impl ResolutionCache for TwoTierCache {
    async fn get(&self, key: &str) -> Option<StreamResolution> {
        if let Some(r) = self.hot.get(key).await {
            if !r.is_expired() {
                self.hits_hot.fetch_add(1, Ordering::Relaxed);
                return Some(r);
            }
            return None;
        }
        let cold = self.cold.get(key).await?;
        if cold.is_expired() {
            return None;
        }
        self.hits_cold.fetch_add(1, Ordering::Relaxed);
        Some(cold)
    }

    async fn put(&self, key: &str, resolution: StreamResolution) {
        self.hot.put(key, resolution.clone()).await;
        self.cold.put(key, resolution).await;
    }

    async fn invalidate(&self, key: &str) {
        self.hot.invalidate(key).await;
        self.cold.invalidate(key).await;
    }

    async fn clear_provider(&self, source: Source) {
        self.hot.clear_provider(source).await;
        self.cold.clear_provider(source).await;
    }

    async fn expiring_within(&self, window: Duration) -> Vec<(String, DateTime<Utc>)> {
        let mut all = self.hot.expiring_within(window).await;
        let seen: std::collections::HashSet<String> = all.iter().map(|(k, _)| k.clone()).collect();
        for (k, exp) in self.cold.expiring_within(window).await {
            if !seen.contains(&k) {
                all.push((k, exp));
            }
        }
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source::Source;

    use chrono::TimeZone;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, h, 0, 0).unwrap()
    }

    fn resolution(uri: &str, expires_at: Option<DateTime<Utc>>) -> StreamResolution {
        StreamResolution {
            expires_at,
            ..StreamResolution::new(Source::YouTube, uri)
        }
    }

    #[tokio::test]
    async fn memory_roundtrip_and_expiry_filtering() {
        let cache = MemoryResolutionCache::default();

        // Sin expiración: vive.
        cache.put("a", resolution("https://cdn/a", None)).await;
        assert!(cache.get("a").await.is_some());

        // Caducada: invisible y purgada.
        cache
            .put("b", resolution("https://cdn/b", Some(t(10))))
            .await;
        assert!(
            cache
                .get("b")
                .await
                .is_none_or(|r| r.uri != "https://cdn/b"),
            "una entrada caducada no se sirve"
        );
        assert!(
            cache.get("b").await.is_none(),
            "la entrada caducada se purga"
        );
    }

    #[tokio::test]
    async fn memory_invalidate_is_key_scoped() {
        let cache = MemoryResolutionCache::default();
        cache.put("yt1", resolution("https://cdn/yt1", None)).await;
        cache.put("yt2", resolution("https://cdn/yt2", None)).await;

        cache.invalidate("yt1").await;
        assert!(cache.get("yt1").await.is_none());
        assert!(
            cache.get("yt2").await.is_some(),
            "invalidar una clave no toca otras"
        );
    }

    #[tokio::test]
    async fn memory_capacity_evicts_oldest_first() {
        let cache = MemoryResolutionCache::new(2);
        for k in ["k1", "k2", "k3"] {
            cache
                .put(k, resolution(format!("https://cdn/{k}").as_str(), None))
                .await;
        }
        assert!(cache.get("k1").await.is_none(), "el más viejo sale primero");
        assert!(cache.get("k2").await.is_some());
        assert!(cache.get("k3").await.is_some());
    }

    #[tokio::test]
    async fn memory_expiring_within_lists_only_near_entries() {
        let now = Utc::now();
        let soon = now + chrono::Duration::minutes(5);
        let far = now + chrono::Duration::hours(5);

        let cache = MemoryResolutionCache::default();
        cache
            .put("soon", resolution("https://c/s", Some(soon)))
            .await;
        cache.put("far", resolution("https://c/f", Some(far))).await;
        cache.put("never", resolution("https://c/n", None)).await;

        let near = cache.expiring_within(Duration::from_secs(600)).await;
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].0, "soon");
    }

    struct CountingCache {
        gets: AtomicUsize,
        puts: AtomicUsize,
    }
    impl CountingCache {
        fn new() -> Self {
            Self {
                gets: AtomicUsize::new(0),
                puts: AtomicUsize::new(0),
            }
        }
    }
    #[async_trait]
    impl ResolutionCache for CountingCache {
        async fn get(&self, _key: &str) -> Option<StreamResolution> {
            self.gets.fetch_add(1, Ordering::Relaxed);
            None
        }
        async fn put(&self, _key: &str, _r: StreamResolution) {
            self.puts.fetch_add(1, Ordering::Relaxed);
        }
        async fn invalidate(&self, _key: &str) {}
        async fn clear_provider(&self, _source: Source) {}
        async fn expiring_within(&self, _w: Duration) -> Vec<(String, DateTime<Utc>)> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn two_tier_reads_through_and_writes_both() {
        let hot = Arc::new(MemoryResolutionCache::default());
        let cold = Arc::new(CountingCache::new());
        let tiered = TwoTierCache::new(hot.clone(), cold.clone());

        assert!(tiered.get("miss").await.is_none());
        assert_eq!(
            cold.gets.load(Ordering::Relaxed),
            1,
            "el miss consulta la capa fría"
        );

        tiered.put("k", resolution("https://cdn/k", None)).await;
        assert_eq!(
            cold.puts.load(Ordering::Relaxed),
            1,
            "put llega a ambas capas"
        );

        assert!(tiered.get("k").await.is_some());
        assert_eq!(
            tiered.hit_stats(),
            (1, 0),
            "tras el put, el hit lo sirve la capa caliente"
        );

        tiered.invalidate("k").await;
        assert!(tiered.get("k").await.is_none());
    }
}
