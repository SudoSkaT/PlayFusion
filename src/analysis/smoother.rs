//! Suavizado de features continuas (EMA con ataque/lanzamiento separados).
//!
//! Separación RAW vs SMOOTHED (spec §19): los valores crudos entran por
//! `step`; el suavizador mantiene su estado interno y devuelve la versión
//! estable. Coeficientes POR SEGUNDO convertidos a factor por frame con
//! `1 - e^(-k·dt)` — independiente del hop size real.

/// Nº de canales continuos suavizados:
/// `[bass, low_mid, mid, high_mid, high, centroid, flux, rms, amplitude]`.
pub const SMOOTHED_CHANNELS: usize = 9;

#[derive(Debug, Clone)]
pub struct FeatureSmoother {
    /// Constante de tiempo al SUBIR (por segundo).
    attack: f32,
    /// Constante de tiempo al BAJAR (por segundo).
    release: f32,
    values: [f32; SMOOTHED_CHANNELS],
}

impl FeatureSmoother {
    pub fn new(attack_per_sec: f32, release_per_sec: f32) -> Self {
        Self {
            attack: attack_per_sec.max(1e-6),
            release: release_per_sec.max(1e-6),
            values: [0.0; SMOOTHED_CHANNELS],
        }
    }

    /// Estado actual (para arrancar un nuevo track sin salto desde cero).
    pub fn current(&self) -> [f32; SMOOTHED_CHANNELS] {
        self.values
    }

    pub fn reset(&mut self) {
        self.values = [0.0; SMOOTHED_CHANNELS];
    }

    /// Avanza un frame de duración `dt` hacia `target`.
    pub fn step(&mut self, target: &[f32; SMOOTHED_CHANNELS], dt: f32) -> [f32; SMOOTHED_CHANNELS] {
        for (slot, &t) in self.values.iter_mut().zip(target.iter()) {
            let t = t.clamp(0.0, 1.0);
            let k = if t > *slot { self.attack } else { self.release };
            let alpha = 1.0 - (-k * dt).exp();
            *slot += (t - *slot) * alpha;
        }
        self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ones() -> [f32; SMOOTHED_CHANNELS] {
        [1.0; SMOOTHED_CHANNELS]
    }

    #[test]
    fn converges_to_target_from_both_directions() {
        let mut sm = FeatureSmoother::new(20.0, 20.0);
        for _ in 0..200 {
            sm.step(&ones(), 1.0 / 86.13);
        }
        assert!(sm.current()[0] > 0.99, "sube hasta el target");

        let zeros = [0.0; SMOOTHED_CHANNELS];
        for _ in 0..200 {
            sm.step(&zeros, 1.0 / 86.13);
        }
        assert!(sm.current()[0] < 0.01, "baja hasta cero");
    }

    #[test]
    fn attack_is_faster_than_release_when_configured() {
        let mut sm = FeatureSmoother::new(30.0, 2.0);

        // Fracción del CAMINO recorrido hacia el target en un frame:
        // subida 0→1 tras el escalón.
        sm.reset();
        sm.step(&ones(), 1.0 / 86.13);
        let up_frac = sm.current()[0]; // distancia 1.0

        // bajada desde saturado hacia 0.
        sm.reset();
        sm.step(&ones(), 60.0); // ~satura en 1
        let start = sm.current()[0];
        sm.step(&[0.0; SMOOTHED_CHANNELS], 1.0 / 86.13);
        let down_frac = (start - sm.current()[0]) / start;

        assert!(
            up_frac > down_frac * 5.0,
            "ataque ({up_frac:.3}) >> release ({down_frac:.3}) en un frame"
        );
    }

    #[test]
    fn zero_dt_is_identity_and_values_are_clamped() {
        let mut sm = FeatureSmoother::new(5.0, 5.0);
        sm.step(&[42.0; SMOOTHED_CHANNELS], 0.0); // dt=0 no cambia
        assert_eq!(sm.current()[3], 0.0);
        // Targets fuera de rango se clampan (features siempre 0..1).
        for _ in 0..100 {
            sm.step(&[-5.0; SMOOTHED_CHANNELS], 0.1);
        }
        assert!(sm.current().iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
