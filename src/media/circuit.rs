//! Circuit breaker por proveedor: evita martillar un provider que ya sabemos
//! que está fallando.
//!
//! Máquina de estados local y sencilla (spec §12):
//!
//! ```text
//! Healthy ──fallos seguidos≥umbral──▶ Open ──cooldown vencido──▶ HalfOpen
//!    ▲                                  │                          │
//!    └──────────── éxito ◀── sonda OK ──┘──── sonda KO ──▶ Open (reabre)
//! ```
//!
//! `Degraded` es el estado informativo intermedio (hay fallos pero aún no se
//! alcanzó el umbral): no bloquea peticiones, solo alimenta métricas.
//!
//! Implementación con `std::sync::Mutex` de secciones críticas mínimas (sin
//! await dentro): el coste por petición es un lock corto, despreciable frente
//! a la resolución de red que protege.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Configuración del breaker.
#[derive(Debug, Clone)]
pub struct CircuitConfig {
    /// Fallos CONSECUTIVOS necesarios para abrir el circuito.
    pub failure_threshold: u32,
    /// Tiempo que permanece abierto antes de permitir una sonda (half-open).
    pub cooldown: Duration,
}

impl Default for CircuitConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            cooldown: Duration::from_secs(30),
        }
    }
}

/// Estado observable del circuito.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Sin fallos recientes: todo pasa.
    Healthy,
    /// Hay fallos consecutivos pero debajo del umbral: sigue pasando todo.
    Degraded,
    /// Circuito abierto: las peticiones se rechazan sin tocar al proveedor.
    Open,
    /// Cooldown vencido: se permite UNA sonda para comprobar recuperación.
    HalfOpen,
}

#[derive(Debug, Default)]
struct Inner {
    consecutive_failures: u32,
    /// `Some(deadline)` mientras el circuito está abierto.
    open_until: Option<Instant>,
    /// Instante en que se concedió la última sonda half-open (`None` si no hay
    /// sonda en vuelo).
    probe_granted_at: Option<Instant>,
}

impl Inner {
    fn is_open(&self, now: Instant) -> bool {
        self.open_until.is_some_and(|until| now < until)
    }
}

/// Breaker por proveedor. Compartible (`Send + Sync`) y barato de clonar vía
/// `Arc` desde el registro.
#[derive(Debug)]
pub struct CircuitBreaker {
    cfg: CircuitConfig,
    inner: Mutex<Inner>,
}

impl CircuitBreaker {
    pub fn new(cfg: CircuitConfig) -> Self {
        Self {
            cfg,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn config(&self) -> &CircuitConfig {
        &self.cfg
    }

    /// Estado actual respecto a `now`.
    ///
    /// Nota: pasar a `HalfOpen` es perezoso (derivado del reloj); no hay tarea
    /// de fondo. El primer `allow_request` tras el cooldown concede la sonda.
    pub fn state(&self, now: Instant) -> CircuitState {
        let g = self.inner.lock().unwrap();
        if g.is_open(now) {
            return CircuitState::Open;
        }
        if g.open_until.is_some() {
            // El cooldown venció: pendiente de sonda.
            return CircuitState::HalfOpen;
        }
        match g.consecutive_failures {
            0 => CircuitState::Healthy,
            _ => CircuitState::Degraded,
        }
    }

    /// `true` si una resolución puede intentar este proveedor ahora mismo.
    ///
    /// En `HalfOpen` concede exactamente una sonda; sondas adicionales (u otra
    /// resolución concurrente) quedan fuera hasta que la sonda resuelva o
    /// caduque (se considera caducada tras `2 × cooldown` sin veredicto, para
    /// que una sonda cancelada no bloquee la recuperación).
    pub fn allow_request(&self, now: Instant) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.is_open(now) {
            return false;
        }
        if g.open_until.is_some() {
            // HalfOpen: ¿hay ya una sonda en vuelo vigente?
            let stale = g
                .probe_granted_at
                .is_some_and(|t| now.duration_since(t) > self.cfg.cooldown * 2);
            if !stale && g.probe_granted_at.is_some() {
                return false;
            }
            g.probe_granted_at = Some(now);
            return true;
        }
        true
    }

    /// Registra un resultado exitoso: cierra el circuito (recuperación).
    pub fn on_success(&self) {
        *self.inner.lock().unwrap() = Inner::default();
    }

    /// Registra un fallo. Si era la sonda half-open, REABRE inmediatamente;
    /// si no, acumula hacia el umbral de apertura.
    pub fn on_failure(&self, now: Instant) {
        let mut g = self.inner.lock().unwrap();
        let was_probing = g.probe_granted_at.take().is_some();
        if was_probing || g.consecutive_failures + 1 >= self.cfg.failure_threshold {
            g.open_until = Some(now + self.cfg.cooldown);
            g.consecutive_failures = self.cfg.failure_threshold;
            return;
        }
        g.consecutive_failures += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: u64) -> Instant {
        Instant::now() + Duration::from_secs(secs)
    }

    fn breaker() -> CircuitBreaker {
        CircuitBreaker::new(CircuitConfig {
            failure_threshold: 3,
            cooldown: Duration::from_secs(30),
        })
    }

    #[test]
    fn healthy_until_threshold_reached() {
        let b = breaker();
        let now = t(0);
        assert_eq!(b.state(now), CircuitState::Healthy);
        assert!(b.allow_request(now));
        b.on_failure(now);
        assert_eq!(b.state(now), CircuitState::Degraded, "bajo umbral: degradado");
        assert!(b.allow_request(now), "degradado NO bloquea");
        b.on_failure(now);
        b.on_failure(now);
        assert_eq!(b.state(now), CircuitState::Open, "tercer fallo abre");
    }

    #[test]
    fn open_rejects_without_touching_provider() {
        let b = breaker();
        let now = t(0);
        for _ in 0..3 {
            b.on_failure(now);
        }
        assert_eq!(b.state(now), CircuitState::Open);
        assert!(!b.allow_request(now), "abierto rechaza");
    }

    #[test]
    fn success_resets_the_circuit() {
        let b = breaker();
        let now = t(0);
        b.on_failure(now);
        b.on_failure(now);
        b.on_success();
        assert_eq!(b.state(now), CircuitState::Healthy, "éxito limpia fallos");
        // Y vuelve a requerir el umbral completo para abrirse.
        for _ in 0..2 {
            b.on_failure(now);
        }
        assert_ne!(b.state(now), CircuitState::Open);
    }

    #[test]
    fn cooldown_leads_to_half_open_with_single_probe() {
        let b = breaker();
        let t0 = t(0);
        for _ in 0..3 {
            b.on_failure(t0);
        }
        let after = t0 + Duration::from_secs(31);
        assert_eq!(b.state(after), CircuitState::HalfOpen);

        // Primera sonda concedida; la segunda (concurrente) rechazada.
        assert!(b.allow_request(after));
        assert!(!b.allow_request(after), "una sola sonda a la vez");

        // Sonda falla → reabre INMEDIATAMENTE (sin esperar más fallos).
        b.on_failure(after);
        assert_eq!(b.state(after), CircuitState::Open);
    }

    #[test]
    fn successful_probe_recovers_to_healthy() {
        let b = breaker();
        let t0 = t(0);
        for _ in 0..3 {
            b.on_failure(t0);
        }
        let after = t0 + Duration::from_secs(31);
        assert!(b.allow_request(after)); // sonda
        b.on_success();
        assert_eq!(b.state(after), CircuitState::Healthy);
        assert!(b.allow_request(after));
    }

    #[test]
    fn abandoned_probe_expires_and_allows_recovery() {
        let b = breaker();
        let t0 = t(0);
        for _ in 0..3 {
            b.on_failure(t0);
        }
        let after = t0 + Duration::from_secs(31);
        assert!(b.allow_request(after)); // sonda concedida y nunca resuelta

        // Sonda abandonada (> 2× cooldown): se puede conceder otra.
        let much_later = after + Duration::from_secs(61);
        assert!(
            b.allow_request(much_later),
            "la sonda abandonada no bloquea la recuperación para siempre"
        );
    }
}
