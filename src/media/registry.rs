//! Registro de proveedores de stream.
//!
//! La aplicación no crea proveedores ad hoc: se registran aquí una sola vez en
//! el punto de composición. El registro posee, por proveedor:
//!
//! - la instancia ([`Arc<dyn StreamProvider>`]);
//! - su flag de habilitado (feature flags / apagado en caliente);
//! - su [`CircuitBreaker`] (salud observada);
//! - un contador de fallos recientes (señal para el router).
//!
//! Las mutaciones son raras (configuración, resultados de resolución); las
//! lecturas ocurren en cada reproducción. Se usa `std::sync::RwLock` con
//! secciones críticas cortas: nunca se retiene el lock a través de un `await`
//! (los snapshots clonan los `Arc`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crate::domain::source::Source;
use crate::media::circuit::{CircuitBreaker, CircuitConfig, CircuitState};
use crate::media::provider::StreamProvider;

/// Entrada interna del registro.
struct Entry {
    provider: Arc<dyn StreamProvider>,
    enabled: bool,
    breaker: Arc<CircuitBreaker>,
    recent_failures: AtomicU32,
}

/// Vista inmutable de una entrada para el router/resolver.
///
/// Clona los `Arc` (barato) para que ninguna decisión dependa del lock.
#[derive(Clone)]
pub struct ProviderSnapshot {
    pub id: &'static str,
    pub source: Source,
    pub priority: u32,
    pub enabled: bool,
    pub breaker: Arc<CircuitBreaker>,
    pub provider: Arc<dyn StreamProvider>,
    /// Fallos recientes acumulados (reset por éxito). Heurística de orden.
    pub recent_failures: u32,
    pub supports_track: bool,
}

impl std::fmt::Debug for ProviderSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSnapshot")
            .field("id", &self.id)
            .field("enabled", &self.enabled)
            .field("priority", &self.priority)
            .finish_non_exhaustive()
    }
}

/// Registro central de proveedores de stream.
#[derive(Default)]
pub struct StreamRegistry {
    entries: RwLock<HashMap<&'static str, Entry>>,
    /// Orden de registro: garantiza planes DETERMINISTAS en empates de
    /// prioridad (HashMap itera en orden aleatorio).
    registration_order: RwLock<Vec<&'static str>>,
}

/// Plan de intento: snapshots ya ordenados (mejor candidato primero).
pub type ProviderPlan = Vec<ProviderSnapshot>;

impl std::fmt::Debug for StreamRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.entries.read().unwrap();
        f.debug_struct("StreamRegistry")
            .field("providers", &g.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra (o reemplaza) un proveedor. El reemplazo conserva el breaker
    /// previo si existe (la salud sobrevive a un re-registro).
    pub fn register(&self, provider: Arc<dyn StreamProvider>) {
        self.register_with_circuit(provider, CircuitConfig::default());
    }

    /// [`Self::register`] con configuración de breaker propia.
    pub fn register_with_circuit(
        &self,
        provider: Arc<dyn StreamProvider>,
        circuit: CircuitConfig,
    ) {
        let id = provider.id();
        let mut g = self.entries.write().unwrap();
        let breaker = g
            .get(id)
            .map(|e| e.breaker.clone())
            .unwrap_or_else(|| Arc::new(CircuitBreaker::new(circuit)));
        let is_new = !g.contains_key(id);
        g.insert(
            id,
            Entry {
                provider,
                enabled: true,
                breaker,
                recent_failures: AtomicU32::new(0),
            },
        );
        drop(g);
        if is_new {
            self.registration_order.write().unwrap().push(id);
        }
    }

    /// Habilita/deshabilita un proveedor. `false` lo saca de toda resolución
    /// futura sin reconstruir nada (apagado en caliente).
    pub fn set_enabled(&self, id: &str, enabled: bool) -> bool {
        let mut g = self.entries.write().unwrap();
        match g.get_mut(id) {
            Some(e) => {
                e.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Snapshot ordenable de todos los proveedores frente a `track`.
    ///
    /// Itera en ORDEN DE REGISTRO (no en el orden aleatorio del mapa) para que
    /// el plan sea determinista; `now` entra como parámetro para que la
    /// evaluación de breakers sea reproducible en tests.
    pub fn snapshot(&self, track: &crate::domain::track::Track, now: Instant) -> Vec<ProviderSnapshot> {
        let g = self.entries.read().unwrap();
        let order = self.registration_order.read().unwrap();
        let mut snaps: Vec<ProviderSnapshot> = order
            .iter()
            .filter_map(|id| g.get(id))
            .filter(|e| e.provider.supports(track))
            .map(|e| ProviderSnapshot {
                id: e.provider.id(),
                source: e.provider.source(),
                priority: e.provider.priority(),
                enabled: e.enabled,
                breaker: e.breaker.clone(),
                provider: e.provider.clone(),
                recent_failures: e.recent_failures.load(Ordering::Relaxed),
                supports_track: true,
            })
            .collect();
        // Orden base estable por prioridad (el router refina encima).
        snaps.sort_by_key(|s| std::cmp::Reverse(s.priority));
        snaps.retain(|s| s.enabled && s.breaker.state(now) != CircuitState::Open);
        snaps
    }

    /// `true` si el breaker concede intentarlo ahora (sonda half-open incluida).
    pub fn allow_attempt(&self, id: &str, now: Instant) -> bool {
        let g = self.entries.read().unwrap();
        g.get(id)
            .map(|e| e.breaker.allow_request(now))
            .unwrap_or(false)
    }

    /// Registra éxito: resetea fallos y cierra el breaker.
    pub fn record_success(&self, id: &str) {
        let g = self.entries.read().unwrap();
        if let Some(e) = g.get(id) {
            e.recent_failures.store(0, Ordering::Relaxed);
            e.breaker.on_success();
        }
    }

    /// Registra fallo: alimenta breaker y contador reciente.
    pub fn record_failure(&self, id: &str) {
        let g = self.entries.read().unwrap();
        if let Some(e) = g.get(id) {
            e.recent_failures.fetch_add(1, Ordering::Relaxed);
            e.breaker.on_failure(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::domain::stream::StreamResolution;
    use crate::domain::track::Track;
    use crate::media::failure::ResolutionError;

    use async_trait::async_trait;

    /// Proveedor falso determinista para tests del registro/router.
    #[derive(Debug)]
    struct FakeProvider {
        id: &'static str,
        source: Source,
        priority: u32,
    }

    #[async_trait]
    impl StreamProvider for FakeProvider {
        fn id(&self) -> &'static str {
            self.id
        }
        fn source(&self) -> Source {
            self.source
        }
        fn priority(&self) -> u32 {
            self.priority
        }
        async fn resolve(
            &self,
            _track: &Track,
            _ctx: &crate::media::provider::ResolveContext,
        ) -> Result<StreamResolution, ResolutionError> {
            Err(ResolutionError::new(
                crate::media::failure::FailureCategory::Unsupported,
                self.source,
                "fake",
            ))
        }
    }

    fn yt_track() -> Track {
        let mut t = Track::new("T".into(), Vec::new(), Source::YouTube);
        t.external_id = Some("vid".into());
        t
    }

    #[test]
    fn snapshot_filters_disabled_unsupported_and_open_breakers() {
        let reg = StreamRegistry::new();
        reg.register(Arc::new(FakeProvider {
            id: "a",
            source: Source::YouTube,
            priority: 200,
        }));
        reg.register(Arc::new(FakeProvider {
            id: "b",
            source: Source::YouTube,
            priority: 100,
        }));
        // "b" deshabilitado sale del snapshot.
        assert!(reg.set_enabled("b", false));

        let track = yt_track();
        let now = Instant::now();
        let snaps = reg.snapshot(&track, now);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].id, "a");
    }

    #[test]
    fn snapshot_orders_by_priority_descending() {
        let reg = StreamRegistry::new();
        for (id, prio) in [("low", 10), ("high", 300), ("mid", 100)] {
            reg.register(Arc::new(FakeProvider {
                id,
                source: Source::YouTube,
                priority: prio,
            }));
        }
        let snaps = reg.snapshot(&yt_track(), Instant::now());
        let ids: Vec<_> = snaps.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["high", "mid", "low"]);
    }

    #[test]
    fn open_breaker_excludes_provider_from_snapshot_until_cooldown() {
        let reg = StreamRegistry::new();
        reg.register_with_circuit(
            Arc::new(FakeProvider {
                id: "flaky",
                source: Source::YouTube,
                priority: 500,
            }),
            CircuitConfig {
                failure_threshold: 2,
                cooldown: Duration::from_secs(30),
            },
        );
        reg.record_failure("flaky");
        reg.record_failure("flaky");

        let now = Instant::now();
        assert!(
            !reg.snapshot(&yt_track(), now).iter().any(|s| s.id == "flaky"),
            "circuito abierto: fuera del plan"
        );

        // Tras el cooldown vuelve (half-open) — el snapshot ya lo lista; la
        // sonda la concede `allow_attempt`.
        let later = now + Duration::from_secs(31);
        assert!(reg.snapshot(&yt_track(), later).iter().any(|s| s.id == "flaky"));
        assert!(reg.allow_attempt("flaky", later), "concede UNA sonda");
        assert!(!reg.allow_attempt("flaky", later), "no concede dos sondas");
    }

    #[test]
    fn success_resets_recent_failures_and_closes_breaker() {
        let reg = StreamRegistry::new();
        reg.register(Arc::new(FakeProvider {
            id: "a",
            source: Source::YouTube,
            priority: 1,
        }));
        reg.record_failure("a");
        reg.record_failure("a");
        reg.record_success("a");
        let snaps = reg.snapshot(&yt_track(), Instant::now());
        assert_eq!(snaps[0].recent_failures, 0);
        assert_eq!(snaps[0].breaker.state(Instant::now()), CircuitState::Healthy);
    }

    #[test]
    fn unknown_ids_are_ignored_safely() {
        let reg = StreamRegistry::new();
        assert!(!reg.set_enabled("ghost", false));
        assert!(!reg.allow_attempt("ghost", Instant::now()));
        reg.record_success("ghost"); // no-op, sin pánico
        reg.record_failure("ghost");
    }
}
