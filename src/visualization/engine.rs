//! VisualEngine: estado visual puro a partir de features + POSICIÓN de
//! reproducción (spec §22).
//!
//! - Sin reloj propio: la fase se calcula `fract(posición × tasa)` — la misma
//!   posición produce SIEMPRE la misma fase (determinista, spec §43).
//! - Suavizado temporal de barras: subida rápida, caída lenta (picos que
//!   respiran); el jitter de turbulencia es función de fase+barras, nunca del
//!   wall-clock.
//! - El pulso decae por EVENTO recibido (~15 Hz de features): determinista
//!   frente al flujo de eventos.

use std::time::Duration;

use crate::analysis::AudioFeatures;
use crate::visualization::palette::VisualPalette;
use crate::visualization::params::{ParameterMapper, VisualParameters};
use crate::visualization::VISUAL_BARS;

/// Nº de "glóbulos" de la escena lava.
pub const VISUAL_BLOBS: usize = 5;

/// Un glóbulo de la lámpara de lava (coordenadas en fracciones del viewport).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blob {
    /// Centro horizontal 0..1 (fracción del ancho interior).
    pub x: f32,
    /// Centro vertical 0..1 (fracción de la altura interior).
    pub y: f32,
    /// Radio 0..1 (fracción de la dimensión interior menor).
    pub r: f32,
}

impl Blob {
    fn lerp(&self, other: &Blob, t: f32) -> Blob {
        Blob {
            x: self.x + (other.x - self.x) * t,
            y: self.y + (other.y - self.y) * t,
            r: self.r + (other.r - self.r) * t,
        }
    }
}

/// Escena ambiental (lámpara de lava) lista para el renderer.
///
/// El App NO conoce metaballs ni blobs: consume `VisualState` y recibe la
/// escena ya calculada por el motor (spec §10).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneState {
    /// Glóbulos en reposo/movimiento (viewport-normalizado, 0..1).
    pub blobs: [Blob; VISUAL_BLOBS],
    /// Energía continua 0..1 (suavizada, RMS).
    pub energy: f32,
    /// Brillo 0..1 (agudos, con aporte del pulso de beat).
    pub brightness: f32,
    /// Deformación 0..1 (medios + flujo; ya aplicada a las posiciones).
    pub distortion: f32,
    /// Paleta fundida del track actual (el renderer solo la consume).
    pub palette: VisualPalette,
    /// `true` cuando hay features frescas (análisis activo y sonando).
    pub active: bool,
}

/// Posiciones/radios de reposo (sin audio): la "lámpara dormida".
pub(crate) fn base_blobs() -> [Blob; VISUAL_BLOBS] {
    [
        Blob {
            x: 0.22,
            y: 0.55,
            r: 0.11,
        },
        Blob {
            x: 0.42,
            y: 0.40,
            r: 0.13,
        },
        Blob {
            x: 0.60,
            y: 0.58,
            r: 0.12,
        },
        Blob {
            x: 0.76,
            y: 0.36,
            r: 0.10,
        },
        Blob {
            x: 0.50,
            y: 0.72,
            r: 0.09,
        },
    ]
}

/// Desfase de fase propio de cada glóbulo (rompe la simetría del paquete).
const BLOB_PHASES: [f32; VISUAL_BLOBS] = [0.0, 1.7, 3.1, 4.6, 2.4];

/// Estado listo para el renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualState {
    /// Alturas finales 0..1 (post jitter y suavizado temporal).
    pub bars: [f32; VISUAL_BARS],
    /// Nivel global 0..1.
    pub level: f32,
    /// Brillo 0..1.
    pub intensity: f32,
    /// Pulso de beat actual 0..1 (decae entre beats).
    pub pulse: f32,
    /// Fase determinista derivada de la posición (0..1).
    pub phase: f32,
    /// `true` cuando hay features frescas (análisis activo y sonando).
    pub active: bool,
    /// Escena ambiental (lava) producida por el mismo motor.
    pub scene: SceneState,
}

impl VisualState {
    pub fn inactive() -> Self {
        Self {
            bars: [0.0; VISUAL_BARS],
            level: 0.0,
            intensity: 0.0,
            pulse: 0.0,
            phase: 0.0,
            active: false,
            scene: SceneState {
                blobs: base_blobs(),
                energy: 0.0,
                brightness: 0.0,
                distortion: 0.0,
                palette: VisualPalette::fallback(),
                active: false,
            },
        }
    }
}

pub struct VisualEngine {
    mapper: ParameterMapper,
    prev_bars: [f32; VISUAL_BARS],
    prev_pulse: f32,
    /// Última posición vista (para detectar seeks: salto brusco ⇒ sin suavizar).
    last_position: Option<Duration>,
    /// Paleta fundida actual (se acerca a la del track en cada frame).
    palette: VisualPalette,
    /// Glóbulos actuales (para la interpolación de seek) y objetivo previo.
    blobs: [Blob; VISUAL_BLOBS],
    /// Snapshot de los glóbulos al detectar el salto (origen del blend).
    prev_blobs: [Blob; VISUAL_BLOBS],
    /// Cuánto queda de la transición de seek (1 al saltar, decae a 0).
    blend: f32,
    /// Envolventes suavizadas de la escena (para que no "tiemblen").
    smooth_energy: f32,
    smooth_brightness: f32,
    smooth_distortion: f32,
}

// Constantes de dinámica (por llamada ≈ cada frame de features ~15 Hz):
const BAR_RISE: f32 = 0.55;
const BAR_FALL: f32 = 0.18;
const PULSE_DECAY: f32 = 0.86;
/// Decaimiento por frame de la transición de seek (≈ cmd a ~0.5 s).
const SCENE_BLEND_DECAY: f32 = 0.60;
/// Cuánto se funde la paleta por frame (consigue la transición de track).
const PALETTE_RATE: f32 = 0.35;
/// Suavizado temporal de los niveles de escena por frame.
const SCENE_SMOOTH: f32 = 0.45;

impl VisualEngine {
    pub fn new(mapper: ParameterMapper) -> Self {
        Self {
            mapper,
            prev_bars: [0.0; VISUAL_BARS],
            prev_pulse: 0.0,
            last_position: None,
            palette: VisualPalette::fallback(),
            blobs: base_blobs(),
            prev_blobs: base_blobs(),
            blend: 0.0,
            smooth_energy: 0.0,
            smooth_brightness: 0.0,
            smooth_distortion: 0.0,
        }
    }

    /// Avanza el estado visual.
    ///
    /// `features` = último snapshot del bus (`None` ⇒ visual inactivo). La
    /// fase usa EXCLUSIVAMENTE `position` — pasarla desde el PositionClock.
    /// `palette` es la paleta del track en curso (del `DecodedThumb`); el motor
    /// la funde internamente con la anterior.
    pub fn update(
        &mut self,
        features: Option<&Arc<AudioFeatures>>,
        position: Duration,
        palette: &VisualPalette,
    ) -> VisualState {
        self.palette = self.palette.mix(palette, PALETTE_RATE);
        let Some(f) = features else {
            // Inactivo: resetear memoria para no arrastrar picos viejos y
            // dejar la escena dormida (reposo) lista para fluir al volver.
            self.prev_bars = [0.0; VISUAL_BARS];
            self.prev_pulse = 0.0;
            self.blobs = base_blobs();
            self.prev_blobs = base_blobs();
            self.smooth_energy = 0.0;
            self.smooth_brightness = 0.0;
            self.smooth_distortion = 0.0;
            self.last_position = None;
            self.blend = 1.0;
            return VisualState {
                bars: [0.0; VISUAL_BARS],
                level: 0.0,
                intensity: 0.0,
                pulse: 0.0,
                phase: 0.0,
                active: false,
                scene: SceneState {
                    blobs: base_blobs(),
                    energy: 0.0,
                    brightness: 0.0,
                    distortion: 0.0,
                    palette: self.palette,
                    active: false,
                },
            };
        };

        let params: VisualParameters = self.mapper.map(f);

        // Seek detection: retroceso grande o salto >2 s reinicia la inercia de
        // barras Y congela el origen de la escena para un blend suave.
        if let Some(prev) = self.last_position {
            let jumped = position < prev || position.saturating_sub(prev) > Duration::from_secs(2);
            if jumped {
                self.prev_bars = params.bars;
                self.prev_pulse = params.pulse_kick;
                self.prev_blobs = self.blobs;
                self.blend = 1.0;
            }
        }
        self.last_position = Some(position);

        // Fase determinista SOLO desde la posición musical.
        let rate = params.phase_rate.max(0.01);
        let phase = (position.as_secs_f32() * rate).fract().clamp(0.0, 1.0);

        // Barras: objetivo con jitter de turbulencia ligado a la fase.
        let mut bars = [0.0f32; VISUAL_BARS];
        for (i, slot) in bars.iter_mut().enumerate() {
            let wobble = ((phase * std::f32::consts::TAU * 3.0) + i as f32 * 0.7).sin();
            let target =
                (params.bars[i] * (1.0 + wobble * 0.18 * params.turbulence)).clamp(0.0, 1.0);
            let k = if target > self.prev_bars[i] {
                BAR_RISE
            } else {
                BAR_FALL
            };
            *slot = self.prev_bars[i] + (target - self.prev_bars[i]) * k;
        }
        self.prev_bars = bars;

        // Pulso: golpe instantáneo + decaimiento exponencial por evento.
        let pulse = (params.pulse_kick).max(self.prev_pulse * PULSE_DECAY);
        self.prev_pulse = pulse;

        // Escena lava: objetivo determinista (posición + parámetros) interpolado
        // desde el origen congelado en el último seek.
        let target = self.compute_blobs(&params, position.as_secs_f32());
        let t = (1.0 - self.blend).clamp(0.0, 1.0);
        let mut blobs = [Blob {
            x: 0.0,
            y: 0.0,
            r: 0.0,
        }; VISUAL_BLOBS];
        for (i, slot) in blobs.iter_mut().enumerate() {
            *slot = self.prev_blobs[i].lerp(&target[i], t);
        }
        self.blobs = blobs;
        self.blend = if self.blend > 0.02 {
            self.blend * SCENE_BLEND_DECAY
        } else {
            0.0
        };

        // Envolventes continuas (no binarias): suben/bajan suaves.
        self.smooth_energy += (params.energy - self.smooth_energy) * SCENE_SMOOTH;
        self.smooth_brightness += (params.brightness - self.smooth_brightness) * SCENE_SMOOTH;
        self.smooth_distortion += (params.distortion - self.smooth_distortion) * SCENE_SMOOTH;

        VisualState {
            bars,
            level: params.level,
            intensity: params.intensity,
            pulse,
            phase,
            active: true,
            scene: SceneState {
                blobs,
                energy: self.smooth_energy.clamp(0.0, 1.0),
                brightness: (self.smooth_brightness + pulse * 0.3).clamp(0.0, 1.0),
                distortion: self.smooth_distortion.clamp(0.0, 1.0),
                palette: self.palette,
                active: true,
            },
        }
    }

    /// Posiciones objetivo de los glóbulos (fracciones 0..1 del viewport).
    ///
    /// Determenístico: función de la posición musical (fase) y de los
    /// parámetros mapeados. Bass → infla (tamaño/impulso), mid+flujo →
    /// deformación del wobble, energy → inflado global, bpm → velocidad de la
    /// fase (a través de `params.phase_rate`).
    fn compute_blobs(&self, params: &VisualParameters, secs: f32) -> [Blob; VISUAL_BLOBS] {
        let freq = secs * params.phase_rate.max(0.01);
        let bass = params.bars[0];
        let inflate = 1.0 + 0.55 * bass + 0.15 * params.energy;
        let mut blobs = base_blobs();
        for (i, b) in blobs.iter_mut().enumerate() {
            let t = freq + BLOB_PHASES[i];
            let wob = (t * 1.3 + params.distortion * 2.5 * (t * 0.7).sin()).sin();
            let drift = (t * 0.5).cos();
            b.x = (b.x + 0.030 * wob + 0.020 * drift).clamp(0.05, 0.95);
            let bounce = 0.030 * (t + 1.7).sin();
            b.y = (b.y + bounce - 0.050 * bass).clamp(0.06, 0.94);
            b.r = (b.r * inflate).clamp(0.03, 0.32);
        }
        blobs
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::bands::BandRatios;
    use crate::visualization::params::{MapperConfig, ParameterMapper};

    fn features(bass: f32, flux: f32, onset: f32, beat: bool, bpm: f32) -> AudioFeatures {
        AudioFeatures {
            timestamp: Duration::from_secs(1),
            rms: bass * 0.5,
            amplitude: bass,
            bass,
            low_mid: bass * 0.5,
            mid: bass * 0.25,
            high_mid: 0.05,
            high: 0.02,
            spectral_centroid: 0.2,
            spectral_flux: flux,
            onset,
            beat,
            beat_confidence: 0.9,
            bpm,
        }
    }

    fn engine() -> VisualEngine {
        VisualEngine::new(ParameterMapper::new(MapperConfig {
            noise_floor: 0.01,
            ..Default::default()
        }))
    }

    const FALLBACK: VisualPalette = VisualPalette::fallback();

    fn pal(cover: Option<[[u8; 3]; 3]>) -> VisualPalette {
        VisualPalette::from_cover(cover)
    }

    #[test]
    fn same_inputs_same_state_deterministic() {
        let mut a = engine();
        let mut b = engine();
        let f = Arc::new(features(0.7, 0.4, 0.5, true, 120.0));
        let pos = Duration::from_secs(42);
        assert_eq!(
            a.update(Some(&f), pos, &FALLBACK),
            b.update(Some(&f), pos, &FALLBACK)
        );
    }

    #[test]
    fn phase_depends_only_on_playback_position_not_wall_clock() {
        let mut e = engine();
        let f = Arc::new(features(0.5, 0.0, 0.0, false, 120.0)); // 2 ciclos/s
        let s1 = e.update(Some(&f), Duration::from_millis(250), &FALLBACK);
        // "Mucho después" en wall-clock pero MISMA posición musical:
        std::thread::sleep(std::time::Duration::from_millis(30));
        let s2 = e.update(Some(&f), Duration::from_millis(250), &FALLBACK);
        assert_eq!(s1.phase, s2.phase, "cero reloj visual independiente");
        assert!(
            (s1.phase - 0.5).abs() < 1e-4,
            "250 ms × 2 ciclos/s → fase ½"
        );
    }

    #[test]
    fn outputs_always_clamped_to_unit_range() {
        let mut e = VisualEngine::new(ParameterMapper::new(MapperConfig {
            sensitivity: 5.0,
            turbulence_gain: 2.0,
            ..Default::default()
        }));
        let loud = Arc::new(features(1.0, 1.0, 1.0, true, 200.0));
        for step in 0..60 {
            let s = e.update(Some(&loud), Duration::from_millis(step * 66), &FALLBACK);
            for v in s.bars.iter() {
                assert!((0.0..=1.0).contains(v), "bar {v} fuera de rango en t{step}");
            }
            for v in [
                s.level,
                s.intensity,
                s.pulse,
                s.scene.energy,
                s.scene.brightness,
                s.scene.distortion,
            ] {
                assert!((0.0..=1.0).contains(&v), "{v} fuera de rango en t{step}");
            }
            assert!((0.0..=1.0).contains(&s.phase));
        }
    }

    #[test]
    fn beat_spikes_pulse_then_decays_monotonically() {
        let mut e = engine();
        let hit = Arc::new(features(0.6, 0.1, 1.0, true, 0.0));
        let rest = Arc::new(features(0.6, 0.1, 0.0, false, 0.0));

        let s0 = e.update(Some(&hit), Duration::ZERO, &FALLBACK);
        assert!(s0.pulse > 0.8, "el beat pega fuerte: {}", s0.pulse);

        let mut prev = s0.pulse;
        for i in 1..12 {
            let s = e.update(Some(&rest), Duration::from_millis(i * 66), &FALLBACK);
            assert!(s.pulse <= prev, "el pulso decae sin nuevos beats");
            prev = s.pulse;
        }
        assert!(prev < 0.25, "y llega casi a cero: {prev}");
    }

    #[test]
    fn silence_is_active_but_low() {
        let mut e = engine();
        let quiet = Arc::new(features(0.005, 0.0, 0.0, false, 0.0));
        let s = e.update(Some(&quiet), Duration::ZERO, &FALLBACK);
        assert!(s.active);
        assert!(s.level < 0.05);
        assert!(s.bars.iter().all(|v| *v < 0.08));
        assert!(s.scene.energy < 0.05);
    }

    #[test]
    fn none_features_resets_to_inactive() {
        let mut e = engine();
        let f = Arc::new(features(0.9, 0.5, 0.5, true, 120.0));
        let _ = e.update(Some(&f), Duration::ZERO, &FALLBACK);
        let off = e.update(None, Duration::ZERO, &FALLBACK);
        assert!(!off.active);
        assert!(off.bars.iter().all(|v| *v == 0.0));
        assert!(!off.scene.active);
        assert_eq!(
            off.scene.blobs,
            base_blobs(),
            "sin features la escena reposa"
        );
        assert_eq!(off.scene.energy, 0.0);

        // Reactivar equivale a un motor FRESCO: sin arrastrar picos viejos.
        let mut virgin = engine();
        let again = e.update(Some(&f), Duration::ZERO, &FALLBACK);
        let baseline = virgin.update(Some(&f), Duration::ZERO, &FALLBACK);
        assert_eq!(again.bars, baseline.bars, "sin memoria de la sesión previa");
    }

    #[test]
    fn seek_reinitializes_bar_inertia_without_crash() {
        let mut e = engine();
        let f = Arc::new(features(0.8, 0.3, 0.0, false, 0.0));
        let _ = e.update(Some(&f), Duration::from_secs(100), &FALLBACK);
        // Retroceso de 90 s: debe comportarse sano (sin pánicos ni NaN).
        let s = e.update(Some(&f), Duration::from_secs(10), &FALLBACK);
        assert!(s.bars.iter().all(|v| v.is_finite()));
        assert!(s
            .scene
            .blobs
            .iter()
            .all(|b| b.x.is_finite() && b.y.is_finite() && b.r.is_finite()));
    }

    fn blob_dist(a: &Blob, b: &Blob) -> f32 {
        let d = |p: f32, q: f32| (p - q).abs();
        d(a.x, b.x) + d(a.y, b.y) + d(a.r, b.r)
    }

    #[test]
    fn seek_blends_scene_toward_target_instead_of_teleporting() {
        let mut e = engine();
        let f = Arc::new(features(0.8, 0.3, 0.0, false, 0.0));
        let _ = e.update(Some(&f), Duration::from_secs(100), &FALLBACK);

        let s_jump = e.update(Some(&f), Duration::from_secs(10), &FALLBACK);
        // El objetivo (posición 10) y el origen (estado en 100) no son idénticos:
        // el seek re-ancla la fase y la escena debe recorrer un tramo real.
        let params = e.mapper.map(&f);
        let target = e.compute_blobs(&params, 10.0);
        let origin = s_jump.scene.blobs;
        let moved: f32 = target
            .iter()
            .zip(origin.iter())
            .map(|(t, o)| blob_dist(t, o))
            .sum();
        assert!(
            moved > 0.05,
            "el seek mueve los glóbulos de forma apreciable: {moved}"
        );

        // Tras suficientes frames en la misma posición, la escena converge al
        // objetivo determinista (transición suave, no parpadeo).
        for _ in 0..40 {
            e.update(Some(&f), Duration::from_secs(10), &FALLBACK);
        }
        let params = e.mapper.map(&f);
        let target = e.compute_blobs(&params, 10.0);
        assert!(
            target
                .iter()
                .zip(e.blobs.iter())
                .all(|(t, o)| blob_dist(t, o) < 0.05),
            "converge al objetivo tras la transición"
        );

        // La convergencia es MONÓTONA en distancia: cada frame se acerca.
        let mut e2 = engine();
        e2.update(Some(&f), Duration::from_secs(100), &FALLBACK);
        e2.update(Some(&f), Duration::from_secs(10), &FALLBACK);
        let params = e2.mapper.map(&f);
        let target = e2.compute_blobs(&params, 10.0);
        let mut dist = blob_dist(&e2.blobs[2], &target[2]);
        for _ in 0..10 {
            e2.update(Some(&f), Duration::from_secs(10), &FALLBACK);
            let d = blob_dist(&e2.blobs[2], &target[2]);
            assert!(d <= dist + 1e-4, "la escena se acerca sin rebotar");
            dist = d;
        }
    }

    #[test]
    fn blobs_stay_in_viewport_bounds_under_extremes() {
        let mut e = VisualEngine::new(ParameterMapper::new(MapperConfig {
            sensitivity: 5.0,
            turbulence_gain: 3.0,
            ..Default::default()
        }));
        let loud = Arc::new(features(1.0, 1.0, 1.0, true, 200.0));
        for step in 0..120 {
            let s = e.update(Some(&loud), Duration::from_millis(step * 66), &FALLBACK);
            for b in s.scene.blobs.iter() {
                assert!((0.0..=1.0).contains(&b.x), "x fuera de viewport: {}", b.x);
                assert!((0.0..=1.0).contains(&b.y), "y fuera de viewport: {}", b.y);
                assert!((0.0..=1.0).contains(&b.r), "r fuera de rango: {}", b.r);
            }
        }
    }

    #[test]
    fn bass_inflates_blobs_and_phase_drifts_them() {
        let mut e = engine();
        let bassy = Arc::new(features(0.95, 0.1, 0.0, false, 0.0));
        let calm = Arc::new(features(0.1, 0.1, 0.0, false, 0.0));
        let s_bass = e.update(Some(&bassy), Duration::from_secs(3), &FALLBACK);
        let mut e = engine();
        let s_calm = e.update(Some(&calm), Duration::from_secs(3), &FALLBACK);
        assert!(
            s_bass
                .scene
                .blobs
                .iter()
                .zip(s_calm.scene.blobs.iter())
                .all(|(a, b)| a.r >= b.r),
            "el bass infla los glóbulos (tamaño/impulso)"
        );
        assert!(
            s_bass.scene.blobs[0] != s_calm.scene.blobs[0],
            "misma posición pero distinta energía ⇒ escena distinta"
        );
        // La fase NO depende del wall-clock: misma posición ⇒ mismo objetivo.
        let mut e3 = engine();
        let a = e3
            .update(Some(&bassy), Duration::from_secs(7), &FALLBACK)
            .scene
            .blobs;
        let b = e3
            .update(Some(&bassy), Duration::from_secs(7), &FALLBACK)
            .scene
            .blobs;
        assert_eq!(a, b, "misma posición ⇒ mismo objetivo (sin reloj propio)");
    }

    #[test]
    fn palette_blends_toward_target_track() {
        let mut e = engine();
        let f = Arc::new(features(0.6, 0.2, 0.0, false, 0.0));
        let warm = pal(Some([[200u8, 40, 40], [40, 200, 60], [30, 60, 220]]));
        // Primeros frames hacia "warm": se acerca, nunca salta.
        let first = e.update(Some(&f), Duration::ZERO, &warm).scene.palette;
        assert_eq!(first, e.palette);
        assert_ne!(first, warm, "la fusión es gradual, no teleport");
        for _ in 0..40 {
            e.update(Some(&f), Duration::from_millis(66), &warm);
        }
        let near_warm = e.palette;
        let d = |a: &VisualPalette, b: &VisualPalette| {
            a.primary
                .iter()
                .zip(b.primary.iter())
                .map(|(x, y)| (*x as i32 - *y as i32).abs())
                .sum::<i32>()
        };
        assert!(d(&near_warm, &warm) <= 3, "converge al track nuevo");
        // Y la escena expone ESA paleta al renderer.
        let s = e.update(Some(&f), Duration::ZERO, &warm);
        assert_eq!(s.scene.palette, e.palette);
    }

    // BandRatios referenciado para mantener import vivo si evoluciona.
    #[allow(dead_code)]
    fn _touch(_: BandRatios) {}
}
