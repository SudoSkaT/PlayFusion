//! Vista Now Playing: tarjeta de canción + barra de progreso + panel de
//! recomendaciones (5 filas visibles, scrollable) + panel de controles.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::audio::{PlaybackState, PlaybackStatus};
use crate::app::thumbnail::ThumbnailState;

use crate::ui::related::RelatedState;
use crate::ui::widgets::{progress_bar, song_card, spinner_phase};
use crate::visualization::render as visualizer;
use crate::visualization::VisualState;

/// Altura del panel de recomendaciones: 5 filas + borde superior/inferior.
const RECS_HEIGHT: u16 = 7;

/// Altura mínima de la tarjeta "Now Playing": deja sitio para la miniatura
/// además de los textos de la canción.
const CARD_HEIGHT_MIN: u16 = 12;
/// Altura máxima de la tarjeta: gana espacio extra cuando el terminal es alto
/// (para que la miniatura pueda crecer hasta ~30 filas) sin aplastar el resto.
const CARD_HEIGHT_MAX: u16 = 34;

#[allow(clippy::too_many_arguments)] // el frame de animación es interno a la vista
pub fn render(
    frame: &mut Frame,
    area: Rect,
    playback: &PlaybackStatus,
    related: &mut RelatedState,
    autoplay: bool,
    mouse: &Option<(u16, u16)>,
    click: &mut bool,
    thumbnails: &std::collections::HashMap<String, ThumbnailState>,
    frame_anim: u64,
    stats: &std::collections::HashMap<String, crate::infrastructure::storage::TrackListeningStats>,
    visual: &VisualState,
) {
    // Banda del visualizador: reservada cuando el terminal tiene altura; con
    // el análisis inactivo se pinta apagada (nunca salta el layout).
    let vis_h: u16 = if area.height >= 24 { 4 } else { 0 };

    let card_h = CARD_HEIGHT_MIN
        .max(area.height.saturating_sub(18 + vis_h))
        .min(CARD_HEIGHT_MAX);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(card_h),
            Constraint::Length(vis_h),
            Constraint::Length(3),
            Constraint::Length(RECS_HEIGHT),
            Constraint::Min(0),
        ])
        .split(area);

    let state = playback
        .track
        .as_ref()
        .and_then(|t| thumbnails.get(&t.identifier()));
    song_card::render(frame, chunks[0], playback.track.as_ref(), state, frame_anim);
    if vis_h > 0 {
        visualizer::render(
            frame,
            chunks[1],
            visual,
            playback.position.as_secs_f32(),
            track_palette(playback, thumbnails),
        );
    }
    progress_bar::render(frame, chunks[2], playback, frame_anim);

    let title = format!(
        " Recomendaciones · {} · a: autoplay {} ",
        related.tracks.len(),
        if autoplay { "ON" } else { "OFF" }
    );
    crate::ui::related::render_tracks_list(
        frame,
        chunks[3],
        related,
        title,
        "Sin recomendaciones todavía. Reproduce una canción o pulsa Enter.",
        mouse,
        click,
        stats,
    );

    render_controls(frame, chunks[4], playback, related, frame_anim);
}

/// Panel de atajos y estado bajo el panel de recomendaciones.
fn render_controls(
    frame: &mut Frame,
    area: Rect,
    playback: &PlaybackStatus,
    related: &RelatedState,
    frame_anim: u64,
) {
    let state = match playback.state {
        PlaybackState::Playing => "▶ reproduciendo",
        PlaybackState::Paused => "⏸ pausado",
        PlaybackState::Stopped => "⏹ detenido",
        PlaybackState::Buffering => "⏳ preparando",
        PlaybackState::Seeking => "🎚 buscando",
    };
    let stall = if playback.stalled {
        format!(
            " · {} red lenta: rellenando buffer…",
            spinner_phase(frame_anim)
        )
    } else {
        String::new()
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("Espacio", Style::new().fg(Color::Cyan)),
            Span::raw(" pausa/reanuda · "),
            Span::styled("←/→", Style::new().fg(Color::Cyan)),
            Span::raw(" ±10s · "),
            Span::styled("↑/↓", Style::new().fg(Color::Cyan)),
            Span::raw(" selección · "),
            Span::styled("Enter", Style::new().fg(Color::Cyan)),
            Span::raw(" reproduce la selección"),
        ]),
        Line::from(vec![
            Span::styled("Shift+D/A", Style::new().fg(Color::Cyan)),
            Span::raw(" siguiente/anterior en la cola · "),
            Span::styled("a", Style::new().fg(Color::Cyan)),
            Span::raw(" alterna autoplay · "),
            Span::styled("Shift+2", Style::new().fg(Color::Cyan)),
            Span::raw(" vista completa con letras"),
        ]),
        Line::from(format!(
            "{state}{stall} · {} recomendaciones cargadas",
            related.tracks.len()
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Controles ")),
        area,
    );
}

/// Paleta de tres colores dominantes de la portada del track en curso, si la
/// miniatura ya está decodificada. Se usa para colorear las barras del visual.
fn track_palette(
    playback: &PlaybackStatus,
    thumbnails: &std::collections::HashMap<String, ThumbnailState>,
) -> Option<[[u8; 3]; 3]> {
    playback
        .track
        .as_ref()
        .and_then(|t| thumbnails.get(&t.identifier()))
        .and_then(|state| match state {
            ThumbnailState::Loaded(img) => img.palette,
            _ => None,
        })
}
