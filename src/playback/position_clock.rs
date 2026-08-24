//! Reloj de posición de reproducción: el audio es la fuente de verdad
//! temporal (spec §17).
//!
//! El motor reporta la posición (`player.get_pos()`, derivada de muestras
//! consumidas) solo cada ~500 ms; este reloj mantiene una lectura continua:
//!
//! - **monótona** dentro del mismo track (un `position` que retrocede es un
//!   reinicio espurio del re-buffer, no un retroceso real del audio);
//! - **extrapolada** mientras se reproduce (última muestra + tiempo real
//!   transcurrido desde ella), congelada en pausa/stall;
//! - **re-anclada en CADA muestra**, incluso si la posición no avanzó: así no
//!   acumula el tiempo de una pausa larga;
//! - **con seek pendiente**: mientras el motor no confirma el salto, sigue el
//!   reloj REAL del audio (que sigue sonando desde donde estaba) y solo al
//!   confirmarse adopta el objetivo.
//!
//! Consumidores: letras sincronizadas/karaoke, análisis y visualización. La
//! lógica fue extraída VERBATIM del reloj del karaoke de `ui/app.rs`; los
//! tests originales cubren ahora este módulo directamente.

use std::time::{Duration, Instant};

/// Seek solicitado aún no confirmado por el motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingSeek {
    pub target: Duration,
}

/// Qué cambió al incorporar una muestra (para que los consumidores reaccionen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockEvent {
    /// Sin track activo: el reloj se apagó (canción terminada/detenida).
    Cleared,
    /// Track nuevo o primera muestra tras limpiar: los consumidores deben
    /// descartar estado dependiente del track anterior (p. ej. letras).
    NewTrack,
}

/// Reloj maestro de posición.
#[derive(Debug, Default)]
pub struct PositionClock {
    /// Identificador estable del track al que pertenece `position`.
    track_key: Option<String>,
    position: Duration,
    seek: Option<PendingSeek>,
    synced_at: Option<Instant>,
}

impl PositionClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Incorpora una muestra del motor.
    ///
    /// `key` es el identificador estable del track reproducido (`None` cuando
    /// el motor reporta paro/cancelación). Devuelve el evento correspondiente
    /// para que la UI limpie lo dependiente del track anterior.
    pub fn update(
        &mut self,
        key: Option<&str>,
        reported: Duration,
        now: Instant,
    ) -> Option<ClockEvent> {
        let Some(key) = key else {
            if self.track_key.is_some() || self.position != Duration::ZERO {
                *self = Self::new();
                return Some(ClockEvent::Cleared);
            }
            return None;
        };

        // Track nuevo: reloj arranca en la posición reportada y los
        // consumidores descartan la letra/estado anterior.
        if self.track_key.as_deref() != Some(key) {
            self.track_key = Some(key.to_string());
            self.position = reported;
            self.synced_at = Some(now);
            self.seek = None;
            return Some(ClockEvent::NewTrack);
        }

        // Seek pendiente: el salto puede tardar algún evento (el motor
        // pre-descarga la región objetivo). Mientras tanto el audio sigue
        // sonando en la posición real reportada; el guard monótono quedaría
        // bloqueando un salto hacia atrás, así que aquí se le hace caso omiso.
        if let Some(seek) = self.seek {
            if reported.abs_diff(seek.target) <= Duration::from_secs(1) {
                // Confirmación del motor: se re-ancla en el OBJETIVO elegido,
                // no en una muestra anterior que llegara por carrera.
                self.position = seek.target;
                self.synced_at = Some(now);
                self.seek = None;
            } else {
                self.position = reported;
                self.synced_at = Some(now);
            }
            return None;
        }

        // Misma canción: el reloj nunca retrocede (reinicio espurio ≠ rewind).
        self.position = self.position.max(reported);
        // Re-anclaje en cada muestra: aunque la posición no avance (pausa,
        // stall), la extrapolación debe partir de AHORA y no de la muestra
        // vieja; si no, al reanudar saltaría todo el tiempo de la pausa.
        self.synced_at = Some(now);
        None
    }

    /// Registra un seek del usuario aún sin confirmar.
    pub fn begin_seek(&mut self, target: Duration) {
        self.seek = Some(PendingSeek { target });
    }

    /// Cancela un seek pendiente (p. ej. llegó otra orden antes de confirmar).
    pub fn cancel_pending_seek(&mut self) {
        self.seek = None;
    }

    /// Replay del MISMO track (autoplay que vuelve a su inicio): rebobina el
    /// reloj a cero SIN cambiar de track — la letra sigue siendo válida.
    pub fn restart_same_track(&mut self) {
        self.position = Duration::ZERO;
        self.synced_at = None;
        self.seek = None;
    }

    /// Apaga el reloj por completo (cambio de canción pedido por el usuario:
    /// nada de la anterior debe sobrevivir).
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Lectura "ahora mismo".
    ///
    /// Mientras reproduce (sin stall) extrapola con el tiempo transcurrido
    /// desde la última muestra; en pausa/stall queda congelada. Nunca supera
    /// `duration` si esta es conocida: la letra no debe "terminar" antes de
    /// tiempo porque el motor dejara de reportar.
    pub fn snapshot(
        &self,
        playing: bool,
        stalled: bool,
        duration: Option<Duration>,
        now: Instant,
    ) -> Duration {
        let value = if playing && !stalled {
            self.synced_at
                .map_or(self.position, |t| self.position + now.saturating_duration_since(t))
        } else {
            self.position
        };
        match duration {
            Some(total) if !total.is_zero() && value > total => total,
            _ => value,
        }
    }

    /// Posición base (sin extrapolación): útil para aserciones y depuración.
    pub fn position(&self) -> Duration {
        self.position
    }

    pub fn track_key(&self) -> Option<&str> {
        self.track_key.as_deref()
    }

    pub fn pending_seek(&self) -> Option<PendingSeek> {
        self.seek
    }

    /// SOLO TESTS: fuerza el instante de anclaje para simular muestras viejas
    /// (p. ej. una pausa larga) sin depender del reloj real.
    #[cfg(test)]
    pub fn force_anchor(&mut self, at: Instant) {
        self.synced_at = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn monotonic_guard_ignores_spurious_resets() {
        let mut c = PositionClock::new();
        let now = t0();
        assert_eq!(c.update(Some("a"), Duration::from_secs(10), now), Some(ClockEvent::NewTrack));
        // Reinicio espurio del motor: ignorado.
        assert_eq!(c.update(Some("a"), Duration::ZERO, now), None);
        assert_eq!(c.position(), Duration::from_secs(10));
        // Y avanza normal después.
        c.update(Some("a"), Duration::from_secs(15), now);
        assert_eq!(c.position(), Duration::from_secs(15));
    }

    #[test]
    fn new_track_event_and_reset_from_reported_position() {
        let mut c = PositionClock::new();
        let now = t0();
        c.update(Some("a"), Duration::from_secs(90), now);
        assert_eq!(
            c.update(Some("b"), Duration::from_secs(3), now),
            Some(ClockEvent::NewTrack),
            "cambiar de track avisa a los consumidores"
        );
        assert_eq!(c.position(), Duration::from_secs(3));
        assert_eq!(c.track_key(), Some("b"));
    }

    #[test]
    fn cleared_when_motor_reports_no_track() {
        let mut c = PositionClock::new();
        let now = t0();
        c.update(Some("a"), Duration::from_secs(5), now);
        assert_eq!(c.update(None, Duration::ZERO, now), Some(ClockEvent::Cleared));
        assert_eq!(c.position(), Duration::ZERO);
        assert_eq!(c.track_key(), None);
        // Un segundo vacío consecutivo no repite el evento.
        assert_eq!(c.update(None, Duration::ZERO, now), None);
    }

    #[test]
    fn pending_seek_follows_real_audio_until_confirmed() {
        let mut c = PositionClock::new();
        let now = t0();
        c.update(Some("a"), Duration::from_secs(100), now);

        // El usuario pide 50s: mientras el motor pre-descarga, el audio sigue
        // en ~100s y el karaoke debe seguirlo (no congelarse en el objetivo).
        c.begin_seek(Duration::from_secs(50));
        c.update(Some("a"), Duration::from_secs(100), now);
        assert_eq!(c.position(), Duration::from_secs(100));
        assert!(c.pending_seek().is_some());

        // El motor llega al objetivo: el seek termina anclado al ELEGIDO.
        c.update(Some("a"), Duration::from_secs(50), now);
        assert_eq!(c.position(), Duration::from_secs(50));
        assert!(c.pending_seek().is_none());

        // Tras resolver, el guard monótono vuelve a operar.
        c.update(Some("a"), Duration::ZERO, now);
        assert_eq!(c.position(), Duration::from_secs(50));
    }

    #[test]
    fn extrapolates_while_playing_freezes_paused_and_clamps_to_duration() {
        let mut c = PositionClock::new();
        let now = t0();
        c.update(Some("a"), Duration::from_secs(42), now);

        // Extrapolación hacia adelante mientras reproduce...
        std::thread::sleep(Duration::from_millis(15));
        let s1 = c.snapshot(true, false, Some(Duration::from_secs(200)), Instant::now());
        assert!(s1 > Duration::from_secs(42), "extrapola entre muestras");

        // ...se congela en pausa...
        assert_eq!(
            c.snapshot(false, false, Some(Duration::from_secs(200)), Instant::now()),
            Duration::from_secs(42)
        );
        // ...y en stall también.
        assert_eq!(
            c.snapshot(true, true, Some(Duration::from_secs(200)), Instant::now()),
            Duration::from_secs(42)
        );

        // Nunca supera la duración conocida: la base YA está en el límite y
        // la extrapolación lo supera → clamp.
        c.update(Some("a"), Duration::from_secs(200), now);
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(
            c.snapshot(true, false, Some(Duration::from_secs(200)), Instant::now()),
            Duration::from_secs(200),
            "clamp a duración"
        );
    }

    #[test]
    fn long_pause_does_not_leak_into_extrapolation() {
        let mut c = PositionClock::new();
        c.update(Some("a"), Duration::from_secs(50), t0());
        // Simula una muestra llegada mucho después CON la misma posición
        // (ticker durante pausa): el re-anclaje evita saltar 10 minutos.
        let later = Instant::now();
        c.update(Some("a"), Duration::from_secs(50), later + Duration::from_secs(600));
        let snap = c.snapshot(false, false, Some(Duration::from_secs(200)), Instant::now());
        assert_eq!(snap, Duration::from_secs(50), "pausa larga no contamina");
    }

    #[test]
    fn restart_same_track_rewinds_but_keeps_identity() {
        let mut c = PositionClock::new();
        let now = t0();
        c.update(Some("a"), Duration::from_secs(120), now);
        c.begin_seek(Duration::from_secs(10));

        c.restart_same_track();
        assert_eq!(c.position(), Duration::ZERO);
        assert!(c.pending_seek().is_none());
        assert_eq!(c.track_key(), Some("a"), "el track NO cambia");

        // La siguiente muestra (posición 1 del replay) se acepta: partía de 0.
        c.update(Some("a"), Duration::from_secs(1), now);
        assert_eq!(c.position(), Duration::from_secs(1));
    }

    #[test]
    fn clear_drops_everything() {
        let mut c = PositionClock::new();
        c.update(Some("a"), Duration::from_secs(9), t0());
        c.begin_seek(Duration::from_secs(2));
        c.clear();
        assert_eq!(c.track_key(), None);
        assert_eq!(c.position(), Duration::ZERO);
        assert!(c.pending_seek().is_none());
    }
}
