//! Política de fallos: decide reintentar o pasar al siguiente proveedor.
//!
//! Es PURA (sin I/O, sin relojes): recibe el error ya clasificado y el nº de
//! intento, y devuelve la acción. El resolver ejecuta la decisión y aplica los
//! topes globales. Así la política es trivialmente testeable y la ejecución
//! (backoff real, cancelación) queda fuera.

use std::time::Duration;

use crate::media::failure::{FailureCategory, ResolutionError};

/// Acción decidida tras un fallo de resolución.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    /// Repetir contra el MISMO proveedor tras esperar `delay`.
    Retry { delay: Duration },
    /// Abandonar este proveedor y pasar al siguiente del plan.
    Fallback,
}

/// Configuración acotada de reintento/fallback.
///
/// Invariantes que el sistema garantiza por diseño:
/// - NUNCA hay fallback infinito (el plan de proveedores es finito).
/// - NUNCA hay retries ilimitados (`retries_per_provider` +
///   `max_total_attempts`).
#[derive(Debug, Clone)]
pub struct FailurePolicy {
    /// Reintentos ADICIONALES contra el mismo proveedor tras el primer fallo
    /// (total de intentos por proveedor = 1 + `retries_per_provider`).
    pub retries_per_provider: u32,
    /// Tope GLOBAL de intentos de red por `resolve()` (todas las rondas de
    /// todos los proveedores sumadas). Anti-cascada cuando hay muchos
    /// proveedores registrados.
    pub max_total_attempts: u32,
    /// Backoff base entre reintentos; crece exponencialmente por intento.
    pub base_backoff: Duration,
    /// Techo del backoff.
    pub max_backoff: Duration,
}

impl Default for FailurePolicy {
    fn default() -> Self {
        Self {
            retries_per_provider: 1,
            max_total_attempts: 6,
            base_backoff: Duration::from_millis(750),
            max_backoff: Duration::from_secs(4),
        }
    }
}

impl FailurePolicy {
    /// Decisión tras un fallo. `attempt` es el nº de intentos YA fallidos
    /// contra este proveedor (0 = primer fallo).
    pub fn decide(&self, err: &ResolutionError, attempt: u32) -> PolicyAction {
        if attempt >= self.retries_per_provider || !err.category.is_retryable() {
            return PolicyAction::Fallback;
        }
        PolicyAction::Retry {
            delay: self.backoff(err.category, attempt),
        }
    }

    /// `true` si se alcanzó el tope global de intentos.
    pub fn total_exhausted(&self, total_attempts: u32) -> bool {
        total_attempts >= self.max_total_attempts
    }

    /// Backoff exponencial: `base × mult^(intento+1)` acotado. El multiplicador
    /// aplica desde el PRIMER reintento; RateLimited espera más agresivamente
    /// (la cuota necesita tiempo), el resto duplica.
    fn backoff(&self, category: FailureCategory, attempt: u32) -> Duration {
        let multiplier: u32 = match category {
            FailureCategory::RateLimited => 4,
            _ => 2,
        };
        let mut d = self.base_backoff;
        for _ in 0..attempt.saturating_add(1) {
            d = d.saturating_mul(multiplier);
            if d >= self.max_backoff {
                return self.max_backoff;
            }
        }
        d.min(self.max_backoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source::Source;
    use crate::media::failure::FailureCategory as C;

    fn err(c: C) -> ResolutionError {
        ResolutionError::new(c, Source::YouTube, "prueba")
    }

    #[test]
    fn retryable_errors_retry_with_backoff() {
        let p = FailurePolicy::default();
        match p.decide(&err(C::NetworkFailure), 0) {
            PolicyAction::Retry { delay } => {
                assert_eq!(delay, Duration::from_millis(1500), "base × 2");
            }
            other => panic!("esperaba Retry, llegó {other:?}"),
        }
    }

    #[test]
    fn non_retryable_errors_fallback_immediately() {
        let p = FailurePolicy::default();
        // Unsupported/InvalidResponse son deterministas: ni un reintento.
        assert_eq!(p.decide(&err(C::Unsupported), 0), PolicyAction::Fallback);
        assert_eq!(p.decide(&err(C::InvalidResponse), 0), PolicyAction::Fallback);
        assert_eq!(
            p.decide(&err(C::ProviderUnavailable), 0),
            PolicyAction::Fallback
        );
    }

    #[test]
    fn per_provider_retries_are_bounded() {
        let p = FailurePolicy {
            retries_per_provider: 2,
            ..FailurePolicy::default()
        };
        assert!(matches!(p.decide(&err(C::Timeout), 0), PolicyAction::Retry { .. }));
        assert!(matches!(p.decide(&err(C::Timeout), 1), PolicyAction::Retry { .. }));
        // Tercer fallo: agotado este proveedor.
        assert_eq!(p.decide(&err(C::Timeout), 2), PolicyAction::Fallback);
        // Y con 0 retries jamás repite.
        let p0 = FailurePolicy {
            retries_per_provider: 0,
            ..FailurePolicy::default()
        };
        assert_eq!(p0.decide(&err(C::NetworkFailure), 0), PolicyAction::Fallback);
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let p = FailurePolicy {
            retries_per_provider: 10,
            ..FailurePolicy::default()
        };
        let d1 = match p.decide(&err(C::NetworkFailure), 0) {
            PolicyAction::Retry { delay } => delay,
            _ => panic!(),
        };
        let d2 = match p.decide(&err(C::NetworkFailure), 1) {
            PolicyAction::Retry { delay } => delay,
            _ => panic!(),
        };
        assert!(d2 > d1, "el backoff crece");
        let d5 = match p.decide(&err(C::NetworkFailure), 5) {
            PolicyAction::Retry { delay } => delay,
            _ => panic!(),
        };
        assert_eq!(d5, p.max_backoff, "el backoff se acota");
    }

    #[test]
    fn rate_limited_waits_longer() {
        let p = FailurePolicy::default();
        let rl = match p.decide(&err(C::RateLimited), 0) {
            PolicyAction::Retry { delay } => delay,
            _ => panic!(),
        };
        let net = match p.decide(&err(C::NetworkFailure), 0) {
            PolicyAction::Retry { delay } => delay,
            _ => panic!(),
        };
        assert!(rl > net, "rate limit espera más que un fallo de red");
    }

    #[test]
    fn global_attempt_cap_is_respected() {
        let p = FailurePolicy::default();
        assert!(!p.total_exhausted(5));
        assert!(p.total_exhausted(6));
        assert!(p.total_exhausted(7));
    }
}
