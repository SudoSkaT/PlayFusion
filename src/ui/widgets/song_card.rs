//! Tarjeta de canción: miniatura (si está disponible) + título, artista,
//! álbum, duración y fuente.

use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::thumbnail::ThumbnailState;
use crate::domain::track::Track;

use super::{format_duration, thumb};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    track: Option<&Track>,
    thumb_state: Option<&ThumbnailState>,
    frame_anim: u64,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Now Playing ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(t) = track else {
        frame.render_widget(
            Paragraph::new(Line::from(
                "Nada en reproducción.\nBusca una canción (Shift+3) y selecciónala con Enter.",
            ))
            .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    };

    // Miniatura a la derecha de la información y grande (hasta 40 columnas si el
    // ancho lo permite): el marco de líneas `|` y su degradado los dibuja
    // `thumb`.
    let thumb_w =
        (40u16.min(inner.width.saturating_sub(12))).clamp(6, inner.width.saturating_sub(2));
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(thumb_w)])
        .split(inner);
    // Un gutter simétrico separa la miniatura tanto del texto como del borde
    // derecho de la tarjeta. El marco y su rectángulo de contenido continúan
    // centrados dentro de esta área real, sin márgenes fijos en la imagen.
    let side_gutter = u16::from(chunks[1].width >= 10);
    let img_area = chunks[1].inner(Margin {
        horizontal: side_gutter,
        vertical: 0,
    });
    let txt_area = chunks[0].inner(Margin {
        horizontal: 0,
        vertical: 0,
    });

    thumb::render_framed(
        frame,
        img_area,
        thumb_state.unwrap_or(&ThumbnailState::None),
        frame_anim,
    );

    let artist = t.primary_artist_name().unwrap_or("Desconocido");
    let album = t.album.as_ref().map(|a| a.title.as_str()).unwrap_or("-");
    let duration = t
        .duration
        .map(format_duration)
        .unwrap_or_else(|| "-".to_string());
    let lines: Vec<Line> = vec![
        Line::styled(
            t.title.clone(),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Line::from(artist.to_string()),
        Line::from(format!("Álbum: {album}")),
        Line::from(format!("Duración: {duration}")),
        Line::styled(
            format!("Fuente: {}", t.source.label()),
            Style::new().fg(Color::Cyan),
        ),
    ];
    // La metadata queda al mismo nivel vertical que la portada: la imagen ya
    // se centra por su marco dentro de `img_area`, así que centrar el texto en
    // `txt_area` alinea ambos bloques a la mitad del cuadro.
    let est_rows: u16 = lines
        .iter()
        .map(|l| {
            let w = l.width().max(1);
            w.div_ceil(txt_area.width as usize).max(1)
        })
        .sum::<usize>() as u16;
    let pad = txt_area.height.saturating_sub(est_rows) / 2;
    let mut centered = Vec::with_capacity(pad as usize + lines.len());
    centered.extend(std::iter::repeat_with(Line::default).take(pad as usize));
    centered.extend(lines);
    frame.render_widget(Paragraph::new(centered).wrap(Wrap { trim: true }), txt_area);
}
