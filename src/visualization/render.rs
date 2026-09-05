//! Renderer TUI de la escena visual: lava ambiental + barras de espectro.
//!
//! Responsabilidad EXCLUSIVA de renderizar (spec §25/§20): sin análisis, sin
//! HTTP, sin providers, sin relojes. Todo lo que pinta está en el estado que
//! recibe. El campo de metaballs se evalúa por celda (sin `exp`, kernel
//! racional barato) y toda la colorimetría sale de [`VisualPalette`] — nunca
//! se deriva aquí.

use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;

use crate::visualization::engine::VisualState;
use crate::visualization::palette::VisualPalette;
use crate::visualization::VISUAL_BARS;

/// Escalera vertical (de abajo hacia arriba). El índice 0 = vacío.
const RAMP: [&str; 8] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇"];

fn to_color(c: [u8; 3]) -> Color {
    Color::Rgb(c[0], c[1], c[2])
}

/// Mezcla lineal de dos colores RGB (pura).
fn mix_c(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2])]
}

/// Oscurece un color por `k`.
fn shade(c: [u8; 3], k: f32) -> [u8; 3] {
    mix_c(c, [0, 0, 0], 1.0 - k.clamp(0.0, 1.0))
}

/// Color de la barra según la intensidad y la paleta fundida de la portada:
/// bajo → acento, medio → secundario, alto → dominante.
fn bar_color(intensity: f32, palette: &VisualPalette) -> Color {
    match (intensity * 4.0) as usize {
        0 => Color::DarkGray,
        1 => to_color(palette.accent),
        2 => to_color(palette.secondary),
        _ => to_color(palette.primary),
    }
}

/// Dibuja la capa ambiental (lámpara de lava) sobre TODO `area`.
///
/// Solo toca el fondo de cada celda (spec: la capa ambiental debe poder vivir
/// detrás de las letras). `subdued` aterriza la escena (reduce resplandor y
/// energía) para que el texto superior siga siendo legible.
pub fn render_ambient(frame: &mut Frame, area: Rect, state: &VisualState, subdued: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let scene = &state.scene;
    let palette = &scene.palette;
    let w = area.width as f32;
    let h = area.height as f32;
    let scale = w.min(h);

    let mut bg = palette.background;
    if subdued {
        bg = shade(bg, 0.45);
    }
    let energy = if subdued || !scene.active {
        0.0
    } else {
        scene.energy
    };
    let brightness = if subdued {
        0.0
    } else if scene.active {
        scene.brightness
    } else {
        0.0
    };
    let distortion = if scene.active { scene.distortion } else { 0.0 };
    let glow = (0.18 + 0.45 * brightness + 0.15 * distortion).clamp(0.0, 1.0);
    let lit_k = if subdued {
        (0.30 + 0.70 * energy) * 0.45
    } else {
        0.30 + 0.70 * energy
    }
    .clamp(0.0, 1.0);

    // Único color derivado por celda: sin allocs por frame.
    let primary = palette.primary;
    let secondary = palette.secondary;
    let accent = palette.accent;

    for row in area.y..area.y + area.height {
        let cy = (row as f32 + 0.5) / h;
        for col in area.x..area.x + area.width {
            let cx = (col as f32 + 0.5) / w;
            let mut dens = 0.0f32;
            for b in scene.blobs.iter() {
                let dx = (cx - b.x) * w;
                let dy = (cy - b.y) * h;
                let r = (b.r * scale).max(0.5);
                let r2 = r * r;
                let d2 = dx * dx + dy * dy;
                if d2 < r2 {
                    let t = 1.0 - d2 / r2;
                    dens += t * t;
                }
            }

            // `lit_k` ya es 0 si la escena está dormida; con densidad 0 la
            // celda queda en el color de fondo plano.
            let mut color = bg;
            if dens > 0.02 {
                let core = dens.sqrt().clamp(0.0, 1.0);
                let inner = if core > 0.45 {
                    mix_c(primary, secondary, (core - 0.45) * 2.2)
                } else {
                    mix_c(bg, accent, core * glow)
                };
                color = mix_c(bg, inner, lit_k);
                if brightness > 0.0 {
                    color = mix_c(color, [255, 255, 255], brightness * 0.12);
                }
            }

            // SAFETY(ninguna): API pública de ratatui; celdas dentro de `area`.
            if let Some(cell) = frame.buffer_mut().cell_mut(Position { x: col, y: row }) {
                cell.set_bg(to_color(color));
            }
        }
    }
}

/// Dibuja el visualizador completo: ambient + barras, con marco de estado.
///
/// Con `state.active == false` pinta un marco apagado sobre la escena dormida
/// (análisis OFF o sin datos aún): la vista nunca "desaparece" ni salta de
/// layout.
pub fn render(frame: &mut Frame, area: Rect, state: &VisualState, position_secs: f32) {
    let pulse_dot = if state.pulse > 0.55 {
        "●"
    } else {
        if state.pulse > 0.2 {
            "◉"
        } else {
            "○"
        }
    };
    let title_color = if state.active {
        bar_color(state.intensity.max(0.15), &state.scene.palette)
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" Visual {} ", pulse_dot),
            Style::new().fg(title_color),
        ))
        .title_bottom(Span::styled(
            format!(" fase {:.2} · pos {:.0}s ", state.phase, position_secs),
            Style::new().fg(Color::DarkGray),
        ));
    frame.render_widget(block, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    render_ambient(frame, inner, state, false);
    render_bars(frame, inner, state);
}

/// Dibuja las barras de espectro sobre la escena ambiental.
fn render_bars(frame: &mut Frame, inner: Rect, state: &VisualState) {
    let inner_w = inner.width as usize;
    let inner_h = inner.height as usize;
    let color = if state.active {
        bar_color(state.intensity, &state.scene.palette)
    } else {
        Color::DarkGray
    };

    for row in 0..inner_h {
        // Distancia desde ARRIBA: una barra "alcanza" esta fila cuando su
        // altura cubre el tramo restante.
        let from_top = (inner_h - 1 - row) as f32;
        let y = inner.y + row as u16;
        for col in 0..inner_w {
            let x = inner.x + col as u16;
            let idx = (col * VISUAL_BARS / inner_w).min(VISUAL_BARS - 1);
            let v = (state.bars[idx] + state.pulse * 0.06).clamp(0.0, 1.0);
            let level = (v * inner_h as f32 - from_top).clamp(0.0, 1.0);
            let step = (level * (RAMP.len() - 1) as f32).round() as usize;

            // SAFETY(ninguna): API pública de ratatui; celdas del área interna.
            if let Some(cell) = frame.buffer_mut().cell_mut(Position { x, y }) {
                cell.set_symbol(RAMP[step]);
                cell.set_style(Style::new().fg(if step > 0 { color } else { Color::DarkGray }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visualization::engine::base_blobs;
    use crate::visualization::VISUAL_BARS;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn active_state(level: f32, palette: VisualPalette) -> VisualState {
        let mut bars = [0.0f32; VISUAL_BARS];
        for (i, b) in bars.iter_mut().enumerate() {
            *b = (level * (1.0 - i as f32 / VISUAL_BARS as f32)).clamp(0.0, 1.0);
        }
        VisualState {
            bars,
            level,
            intensity: level,
            pulse: level * 0.8,
            phase: 0.25,
            active: true,
            scene: crate::visualization::engine::SceneState {
                blobs: base_blobs(),
                energy: level,
                brightness: level,
                distortion: level * 0.5,
                palette,
                active: true,
            },
        }
    }

    fn drawing(ch: &dyn Fn(&mut Frame)) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(ch).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Suma de difidencia del fondo de una celda respecto al fondo plano de la
    /// paleta: cuánto "tinte de lava" hay en esa celda.
    fn tint_of(c: ratatui::style::Color, base: [u8; 3]) -> u32 {
        match c {
            Color::Rgb(r, g, b) => {
                (r as i32 - base[0] as i32).unsigned_abs()
                    + (g as i32 - base[1] as i32).unsigned_abs()
                    + (b as i32 - base[2] as i32).unsigned_abs()
            }
            _ => 0,
        }
    }

    /// Máximo tinte de lava sobre el plano en todo el buffer.
    fn max_tint(buf: &ratatui::buffer::Buffer, base: [u8; 3]) -> u32 {
        buf.content()
            .iter()
            .map(|c| tint_of(c.bg, base))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn renders_active_and_inactive_without_panic() {
        let backend = TestBackend::new(60, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &active_state(0.9, VisualPalette::fallback()),
                    42.0,
                )
            })
            .unwrap();
        terminal
            .draw(|f| render(f, f.area(), &VisualState::inactive(), 0.0))
            .unwrap();
        terminal
            .draw(|f| render_ambient(f, f.area(), &VisualState::inactive(), true))
            .unwrap();
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        let backend = TestBackend::new(10, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &active_state(1.0, VisualPalette::fallback()),
                    1.0,
                )
            })
            .unwrap();
        terminal
            .draw(|f| {
                render_ambient(
                    f,
                    Rect::new(0, 0, 5, 2),
                    &active_state(1.0, VisualPalette::fallback()),
                    false,
                )
            })
            .unwrap();
        terminal
            .draw(|f| render_ambient(f, Rect::new(0, 0, 0, 0), &VisualState::inactive(), false))
            .unwrap();
    }

    #[test]
    fn louder_state_paints_more_filled_cells() {
        let case = |level: f32| {
            let buf = drawing(&|f| {
                render(
                    f,
                    f.area(),
                    &active_state(level, VisualPalette::fallback()),
                    0.0,
                )
            });
            buf.content()
                .iter()
                .filter(|c| !c.symbol().trim().is_empty())
                .count()
        };
        assert!(case(0.9) > case(0.15), "más nivel ⇒ más celdas pintadas");
    }

    #[test]
    fn palette_colors_the_bars() {
        // Con paleta, una barra activa usa un color RGB de la portada en vez
        // del esquema cian fijo: intensidad alta → dominante, leve → acento.
        let cover = Some([[220u8, 30, 30], [40, 200, 60], [30, 60, 220]]);
        // Intensidad 0.9: `(0.9*4)=3` → tier del dominante (primary).
        let high = active_state(0.9, VisualPalette::from_cover(cover));
        let buf = drawing(&|f| render(f, f.area(), &high, 0.0));
        let has_palette_color = buf
            .content()
            .iter()
            .any(|c| c.fg == Color::Rgb(220, 30, 30));
        assert!(has_palette_color, "las barras usan colores de la portada");
        // Intensidad 0.3: `(0.3*4)=1` → tier del acento (tercero).
        let low = active_state(0.3, VisualPalette::from_cover(cover));
        let buf = drawing(&|f| render(f, f.area(), &low, 0.0));
        let has_accent = buf
            .content()
            .iter()
            .any(|c| c.fg == Color::Rgb(30, 60, 220));
        assert!(has_accent, "una barra leve usa el acento de la portada");
    }

    /// Tinte máximo de lava del buffer, `None` si es negligible (fondo plano).
    fn first_lit_cell(buf: &ratatui::buffer::Buffer, state: &VisualState) -> Option<u32> {
        let base = state.scene.palette.background;
        let t = max_tint(buf, base);
        (t > 0).then_some(t)
    }

    #[test]
    fn more_energy_brighter_lava() {
        let palette =
            VisualPalette::from_cover(Some([[200u8, 30, 80], [40, 160, 240], [240, 180, 80]]));
        let mut low_s = active_state(0.05, palette);
        low_s.scene.energy = 0.0;
        let mut high = active_state(1.0, palette); // energía alta
        high.scene.brightness = 0.0;

        let buf_low = drawing(&|f| render_ambient(f, f.area(), &low_s, false));
        let buf_high = drawing(&|f| render_ambient(f, f.area(), &high, false));
        let lit_low = first_lit_cell(&buf_low, &low_s).unwrap_or(0);
        let lit_high = first_lit_cell(&buf_high, &high).unwrap_or(0);
        assert!(
            lit_high > 0,
            "con energía hay lava teñida por encima del plano"
        );
        assert!(lit_high > lit_low, "más energía ⇒ más tinte de lava");
    }

    #[test]
    fn subdued_dims_the_lava_for_legibility() {
        let palette = VisualPalette::fallback();
        let st = active_state(0.9, palette);
        let full = drawing(&|f| render_ambient(f, f.area(), &st, false));
        let dim = drawing(&|f| render_ambient(f, f.area(), &st, true));
        let sum_rgb = |c: Color| -> u32 {
            match c {
                Color::Rgb(r, g, b) => u32::from(r) + u32::from(g) + u32::from(b),
                _ => 0,
            }
        };
        let dimmer = full
            .content()
            .iter()
            .zip(dim.content().iter())
            .all(|(c1, c2)| sum_rgb(c2.bg) <= sum_rgb(c1.bg));
        assert!(dimmer, "sin excepción, la lava aplacada es más oscura");
        assert!(
            dim.content()
                .iter()
                .zip(full.content().iter())
                .any(|(c2, c1)| sum_rgb(c2.bg) < sum_rgb(c1.bg)),
            "y al menos una celda se atenúa de verdad"
        );
    }

    #[test]
    fn ambient_only_sets_background_not_symbols() {
        let st = active_state(0.9, VisualPalette::fallback());
        let buf = drawing(&|f| render_ambient(f, f.area(), &st, false));
        // La capa ambiental no pinta glifos (solo fondo), así el texto superior
        // (karaoke) puede superponerse limpiamente.
        assert!(
            buf.content().iter().all(|c| c.symbol() == " "),
            "ambient es fondo puro (celda en blanco, sin símbolos)"
        );
    }
}
