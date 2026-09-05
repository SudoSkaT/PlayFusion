//! Paleta visual derivada de la portada (spec §4).
//!
//! El track en curso ya produce tres colores dominantes (`DecodedThumb.palette`,
//! `[[u8;3];3]`). Aquí se formalizan como [`VisualPalette`], un CONTRATO propio
//! de la capa de visualización: el motor visual la recibe y la funde con la de
//! la canción anterior (transición de cambio de track), y el renderer solo la
//! consume. La extracción nunca ocurre en el renderer.
//!
//! Pura y determinista: sin aleatoriedad, sin estado, sin dependencia del
//! renderer (solo `[u8;3]`).

/// Tres colores dominantes + fondo derivado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualPalette {
    /// Color más dominante (barras altas, línea de karaoke en lectura).
    pub primary: [u8; 3],
    /// Segundo dominante (barras medias, karaoke ya leído).
    pub secondary: [u8; 3],
    /// Tercer dominante (barras bajas, karaoke no leído).
    pub accent: [u8; 3],
    /// Fondo de escena: versión oscurecida del dominante.
    pub background: [u8; 3],
}

impl VisualPalette {
    /// Paleta fija para cuando no hay portada (determinista, sin `rand`).
    pub const fn fallback() -> Self {
        Self {
            primary: [226, 120, 224],
            secondary: [96, 168, 252],
            accent: [250, 176, 96],
            background: [20, 14, 26],
        }
    }

    /// De los tres colores dominantes de la portada (`None` ⇒ [`Self::fallback`]).
    pub fn from_cover(cover: Option<[[u8; 3]; 3]>) -> Self {
        let Some(p) = cover else {
            return Self::fallback();
        };
        Self {
            primary: p[0],
            secondary: p[1],
            accent: p[2],
            background: Self::shade(p[0], 0.22),
        }
    }

    /// Mezcla lineal hacia `other` (t=0 mantiene `self`, t=1 llega a `other`).
    ///
    /// Pura y determinista: el motor la usa una vez por frame para fundir la
    /// paleta del track anterior con la nueva.
    pub fn mix(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self {
            primary: [
                lerp(self.primary[0], other.primary[0]),
                lerp(self.primary[1], other.primary[1]),
                lerp(self.primary[2], other.primary[2]),
            ],
            secondary: [
                lerp(self.secondary[0], other.secondary[0]),
                lerp(self.secondary[1], other.secondary[1]),
                lerp(self.secondary[2], other.secondary[2]),
            ],
            accent: [
                lerp(self.accent[0], other.accent[0]),
                lerp(self.accent[1], other.accent[1]),
                lerp(self.accent[2], other.accent[2]),
            ],
            background: [
                lerp(self.background[0], other.background[0]),
                lerp(self.background[1], other.background[1]),
                lerp(self.background[2], other.background[2]),
            ],
        }
    }

    /// Oscurece un color por `k` (0..1).
    fn shade(c: [u8; 3], k: f32) -> [u8; 3] {
        [
            (c[0] as f32 * k).round().clamp(0.0, 255.0) as u8,
            (c[1] as f32 * k).round().clamp(0.0, 255.0) as u8,
            (c[2] as f32 * k).round().clamp(0.0, 255.0) as u8,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cover_equals_fallback_exactly() {
        assert_eq!(VisualPalette::from_cover(None), VisualPalette::fallback());
    }

    #[test]
    fn from_cover_maps_dominants_and_darkens_background() {
        let cover = Some([[200u8, 40, 40], [40, 200, 60], [30, 60, 220]]);
        let p = VisualPalette::from_cover(cover);
        assert_eq!(p.primary, [200, 40, 40]);
        assert_eq!(p.secondary, [40, 200, 60]);
        assert_eq!(p.accent, [30, 60, 220]);
        assert_eq!(p.background, [(200.0f32 * 0.22).round() as u8, 9, 9]);
        assert!(
            u32::from(p.background[0]) + u32::from(p.background[1]) + u32::from(p.background[2])
                < u32::from(p.primary[0]) + u32::from(p.primary[1]) + u32::from(p.primary[2]),
            "el fondo es siempre una versión oscura del dominante"
        );
    }

    #[test]
    fn mix_endpoints_are_exact() {
        let a = VisualPalette::from_cover(Some([[10u8, 10, 10], [20, 20, 20], [30, 30, 30]]));
        let b =
            VisualPalette::from_cover(Some([[200u8, 200, 200], [220, 220, 220], [240, 240, 240]]));
        assert_eq!(a.mix(&b, 0.0), a);
        assert_eq!(a.mix(&b, 1.0), b);
    }

    #[test]
    fn mix_is_deterministic_and_bounded() {
        let a = VisualPalette::fallback();
        let b = VisualPalette::from_cover(Some([[0u8, 20, 250], [10, 30, 0], [5, 5, 5]]));
        let m1 = a.mix(&b, 0.43);
        let m2 = a.mix(&b, 0.43);
        assert_eq!(m1, m2, "la mezcla es una función pura");
        // t fuera de rango se satura a 0/1 (sin NaN ni desbordes).
        assert_eq!(a.mix(&b, -5.0), a, "t < 0 se trata como 0");
        assert_eq!(a.mix(&b, 99.0), b, "t > 1 se trata como 1");
    }

    #[test]
    fn repeated_mix_converges_toward_target() {
        let target = VisualPalette::from_cover(Some([[255u8, 0, 0], [0, 255, 0], [0, 0, 255]]));
        let mut cur = VisualPalette::fallback();
        for _ in 0..40 {
            cur = cur.mix(&target, 0.35);
        }
        let diff = |a: &[u8; 3], b: &[u8; 3]| {
            a.iter()
                .zip(b.iter())
                .map(|(x, y)| (*x as i32 - *y as i32).abs())
                .sum::<i32>()
        };
        // ≤3 por canal admite el residuo de redondeo u8 al converger.
        assert!(
            diff(&cur.primary, &target.primary) <= 3,
            "converge al dominante"
        );
        assert!(diff(&cur.background, &target.background) <= 3);
    }
}
