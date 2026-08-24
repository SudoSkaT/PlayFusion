//! Renderer TUI del visualizador: dibuja un [`VisualState`].
//!
//! Responsabilidad EXCLUSIVA de renderizar (spec §25): sin análisis, sin
//! HTTP, sin providers, sin relojes. Todo lo que pinta está en el estado que
//! recibe.

use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;

use crate::visualization::engine::VisualState;
use crate::visualization::VISUAL_BARS;

/// Escalera vertical (de abajo hacia arriba). El índice 0 = vacío.
const RAMP: [&str; 8] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇"];

/// Color por intensidad: cian frío → magenta caliente.
fn heat_color(intensity: f32) -> Color {
    match (intensity * 4.0) as usize {
        0 => Color::DarkGray,
        1 => Color::Cyan,
        2 => Color::Blue,
        3 => Color::Magenta,
        _ => Color::LightMagenta,
    }
}

/// Dibuja el visualizador en `area`.
///
/// Con `state.active == false` pinta un marco apagado (análisis OFF o sin
/// datos aún): la vista nunca "desaparece" ni salta de layout.
pub fn render(frame: &mut Frame, area: Rect, state: &VisualState, position_secs: f32) {
    let pulse_dot = if state.pulse > 0.55 { "●" } else { if state.pulse > 0.2 { "◉" } else { "○" } };
    let title_color = if state.active { heat_color(state.intensity.max(0.15)) } else { Color::DarkGray };

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

    // Dibujo DIRECTO al buffer (spec §34: cero allocations por frame — ni
    // Vec<Line> ni Spans temporales; una escritura de celda por posición).
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let inner_w = inner.width as usize;
    let inner_h = inner.height as usize;
    if inner_w == 0 || inner_h == 0 {
        return;
    }

    let color = if state.active {
        heat_color(state.intensity)
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
    use crate::visualization::VISUAL_BARS;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn active_state(level: f32) -> VisualState {
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
        }
    }

    #[test]
    fn renders_active_and_inactive_without_panic() {
        let backend = TestBackend::new(60, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &active_state(0.9), 42.0)).unwrap();
        terminal.draw(|f| render(f, f.area(), &VisualState::inactive(), 0.0)).unwrap();
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        let backend = TestBackend::new(10, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &active_state(1.0), 1.0)).unwrap();
        // Área degenerada de 0 filas útiles:
        terminal
            .draw(|f| render(f, Rect::new(0, 0, 5, 2), &active_state(1.0), 1.0))
            .unwrap();
    }

    #[test]
    fn louder_state_paints_more_filled_cells() {
        let case = |level: f32| {
            let backend = TestBackend::new(40, 6);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|f| render(f, f.area(), &active_state(level), 0.0)).unwrap();
            let content = terminal.backend().buffer().content().iter()
                .filter(|c| !c.symbol().trim().is_empty())
                .count();
            content
        };
        assert!(case(0.9) > case(0.15), "más nivel ⇒ más celdas pintadas");
    }
}
