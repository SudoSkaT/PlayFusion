//! Recuperación de fallos en caliente (spec §37).
//!
//! Flujo objetivo:
//!
//! ```text
//! motor detecta fallo → clasificar → invalidar resolución si procede
//!   → re-resolver (FRESCO) → re-preparar → continuar si es posible
//! ```
//!
//! Sin reiniciar la aplicación, sin romper la cola. El presupuesto es de UN
//! intento por track: un stream que muere dos veces seguidas no se martilla
//! (el usuario ya ve el aviso; el siguiente track de la cola sigue su curso).
//!
//! Decisiones documentadas:
//! - `Cut` (límite del CDN) mantiene su comportamiento actual: aviso + fin
//!   natural del prefijo. Re-lanzar automáticamente en una red YA limitada
//!   solo multiplicaría cortes; la renovación manual/replay sí pasa por el
//!   resolver.
//! - Fallos de decodificación o del dispositivo NO se recuperan con otra
//!   URL: no son problemas de resolución.

use crate::app::audio::PlaybackEvent;

/// Acción decidida ante un evento adverso en caliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Invalidar y re-resolver el stream del track actual, luego continuar.
    RefreshAndResume,
    /// Solo informar (aviso discreto); sin reintento automático.
    Report,
}

/// Clasifica un evento de reproducción y decide la recuperación.
pub fn decide_recovery(event: &PlaybackEvent) -> RecoveryAction {
    match event {
        PlaybackEvent::Error(msg) if is_transport_failure(msg) => RecoveryAction::RefreshAndResume,
        _ => RecoveryAction::Report,
    }
}

/// `true` si un mensaje de error en caliente apunta a un problema de
/// TRANSPORTE (la URL murió / la red falló): una resolución fresca puede
/// arreglarlo.
fn is_transport_failure(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    // Cancelaciones propias y órdenes reemplazadas: no son fallos.
    if m.contains("cancelad") || m.contains("reemplazad") {
        return false;
    }
    // Decodificación y dispositivo: cambiar de URL no los arregla.
    !m.contains("decodificar") && !m.contains("dispositivo")
}

/// Presupuesto de auto-recuperación: UN refresco por track.
///
/// Se arma al arrancar cada reproducción; `try_consume` gasta el único
/// intento. Cambiar de track rearma automáticamente.
#[derive(Debug, Default)]
pub struct RecoveryBudget {
    armed_key: Option<String>,
    consumed: bool,
}

impl RecoveryBudget {
    /// Arma (o rearma) el presupuesto para el track `key`.
    pub fn arm(&mut self, key: &str) {
        *self = Self {
            armed_key: Some(key.to_string()),
            consumed: false,
        };
    }

    /// Gasta el intento para `key`. `false` si el track no coincide o ya se
    /// gastó.
    pub fn try_consume(&mut self, key: &str) -> bool {
        if self.armed_key.as_deref() == Some(key) && !self.consumed {
            self.consumed = true;
            return true;
        }
        false
    }

    #[cfg(test)]
    pub fn remaining(&self, key: &str) -> bool {
        self.armed_key.as_deref() == Some(key) && !self.consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_failures_trigger_refresh_and_resume() {
        for msg in [
            "leer stream: conexión interrumpida",
            "descargar bloque 4096: 403 Forbidden",
            "stream interrumpido",
            "el stream respondió 502",
        ] {
            assert_eq!(
                decide_recovery(&PlaybackEvent::Error(msg.to_string())),
                RecoveryAction::RefreshAndResume,
                "{msg} es transporte"
            );
        }
    }

    #[test]
    fn decode_device_and_cancellations_are_report_only() {
        for msg in [
            "decodificar stream: cabecera inválida",
            "dispositivo de audio desconectado",
            "buffer de audio del dispositivo agotado (underrun)",
            "reproducción cancelada",
            "reproducción reemplazada por otra orden",
        ] {
            assert_eq!(
                decide_recovery(&PlaybackEvent::Error(msg.to_string())),
                RecoveryAction::Report,
                "{msg} no se arregla re-resolviendo"
            );
        }
    }

    #[test]
    fn cut_keeps_its_current_semantics() {
        assert_eq!(
            decide_recovery(&PlaybackEvent::Cut(
                "el servidor restringe este stream a partir del byte 1048576".to_string()
            )),
            RecoveryAction::Report,
            "una restricción del servidor no se arregla re-lanzando la misma URL"
        );
    }

    #[test]
    fn benign_events_never_recover() {
        for ev in [
            PlaybackEvent::Buffering,
            PlaybackEvent::Playing,
            PlaybackEvent::Paused,
            PlaybackEvent::Finished,
            PlaybackEvent::Stopped,
        ] {
            assert_eq!(decide_recovery(&ev), RecoveryAction::Report);
        }
    }

    #[test]
    fn budget_allows_exactly_one_refresh_per_track() {
        let mut budget = RecoveryBudget::default();
        budget.arm("song-a");
        assert!(budget.remaining("song-a"));

        assert!(budget.try_consume("song-a"), "primer intento concedido");
        assert!(!budget.try_consume("song-a"), "segundo intento denegado");
        assert!(!budget.try_consume("song-b"), "clave ajena denegada");
    }

    #[test]
    fn arming_a_new_track_rearms_the_budget() {
        let mut budget = RecoveryBudget::default();
        budget.arm("a");
        assert!(budget.try_consume("a"));
        budget.arm("b");
        assert!(budget.try_consume("b"), "track nuevo = presupuesto nuevo");
        assert!(!budget.try_consume("a"), "el viejo sigue gastado");
    }
}
