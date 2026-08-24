//! Router de proveedores: decide el ORDEN de intento.
//!
//! Es una función PURA sobre snapshots del registro: sin I/O, sin relojes, sin
//! conocimiento de proveedores concretos. Considera (spec §10):
//!
//! 1. capacidad (`supports`, ya filtrada en el snapshot);
//! 2. disponibilidad (habilitado y breaker NO abierto, ya filtrados);
//! 3. prioridad estática declarada por cada provider;
//! 4. historial reciente de fallos (menos fallos primero a igualdad);
//! 5. configuración/feature flags (entran vía `enabled` en el snapshot).
//!
//! NUNCA hay lógica `if provider == youtube` aquí: las particularidades de un
//! proveedor viven en su adaptador.

use crate::domain::track::Track;
use crate::media::registry::{ProviderPlan, ProviderSnapshot};

/// Ordena los candidatos para intentar resolver `track`.
///
/// Criterios en cascada:
/// - prioridad descendente (declaración estática del provider);
/// - a igualdad, menos fallos recientes primero;
/// - a igualdad total, orden estable (determinismo en tests y logs).
///
/// Los candidatos con circuito abierto o deshabilitados YA fueron excluidos
/// por [`crate::media::StreamRegistry::snapshot`]; este módulo no los re-añade.
pub fn order(track: &Track, snapshots: Vec<ProviderSnapshot>) -> ProviderPlan {
    let _ = track; // hoy el plan no depende del track más allá del filtrado previo
    let mut plan = snapshots;
    plan.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.recent_failures.cmp(&b.recent_failures))
    });
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source::Source;
    use crate::media::circuit::{CircuitBreaker, CircuitConfig};
    use std::sync::Arc;

    fn snap(id: &'static str, priority: u32, failures: u32) -> ProviderSnapshot {
        ProviderSnapshot {
            id,
            source: Source::YouTube,
            priority,
            enabled: true,
            breaker: Arc::new(CircuitBreaker::new(CircuitConfig::default())),
            provider: Arc::new(crate::media::test_support::NullProvider(
                id,
                Source::YouTube,
            )),
            recent_failures: failures,
            supports_track: true,
        }
    }

    #[test]
    fn orders_by_priority_then_fewest_recent_failures() {
        let track = Track::new("T".into(), Vec::new(), Source::YouTube);
        let plan = order(
            &track,
            vec![
                snap("low-clean", 10, 0),
                snap("high-dirty", 100, 5),
                snap("high-clean", 100, 0),
                snap("mid", 50, 2),
            ],
        );
        let ids: Vec<_> = plan.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["high-clean", "high-dirty", "mid", "low-clean"]);
    }

    #[test]
    fn equal_candidates_keep_insertion_order() {
        let track = Track::new("T".into(), Vec::new(), Source::YouTube);
        let plan = order(&track, vec![snap("first", 100, 1), snap("second", 100, 1)]);
        assert_eq!(plan[0].id, "first");
        assert_eq!(plan[1].id, "second");
    }
}
