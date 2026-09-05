//! Vista Related: letras (modo karaoke con LRC de LRCLIB) + recomendaciones.
//!
//! Muestra las letras sincronizadas de la canción en curso con la línea activa
//! resaltada (recibe la posición de reproducción de la UI); si no hay LRC se
//! muestra un estado limpio — nunca letra plana. Debajo, una lista de `tracks`
//! relacionadas.
//!
//! [`render_tracks_list`] es un helper compartido: la vista Now Playing lo usa
//! para el panel de recomendaciones de su pantalla principal.

use std::time::Duration;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::domain::lyrics::SyncLyrics;
use crate::domain::track::Track;
use crate::infrastructure::storage::TrackListeningStats;

use super::widgets::karaoke::KaraokeScroller;
use crate::visualization::render as visualizer;

use super::navigation::ListSelection;

#[derive(Debug, Default)]
pub struct RelatedState {
    pub tracks: Vec<Track>,
    /// Letra sincronizada (LRC, LRCLIB) para el modo karaoke: única fuente.
    pub synced: Option<SyncLyrics>,
    /// `true` cuando ya se intentó cargar la letra sincronizada y no existe:
    /// se muestra un estado limpio en vez de la letra plana antigua.
    pub synced_unavailable: bool,
    /// Ventana del karaoke: buffer circular de índices con desplazamiento en
    /// cascada sobre [`Self::synced`]. Se reinicia al cambiar de letra.
    pub scroll: KaraokeScroller,
    pub list_state: ListState,
}

impl RelatedState {
    pub fn has_tracks(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// Nueva canción: se descartan las letras anteriores y se vuelve al estado
    /// "sin pedir" (la ventana del karaoke es de la canción vieja y no debe
    /// reutilizarse nunca).
    pub fn clear_lyrics(&mut self) {
        self.scroll.reset();
        self.synced = None;
        self.synced_unavailable = false;
    }

    /// Resultado de la carga de letras sincronizadas:
    /// `Some` → karaoke listo; `None` → no hay LRC (estado limpio de aviso).
    pub fn set_synced(&mut self, synced: Option<SyncLyrics>) {
        self.scroll.reset();
        match synced {
            Some(s) => {
                self.synced = Some(s);
                self.synced_unavailable = false;
            }
            None => {
                self.synced = None;
                self.synced_unavailable = true;
            }
        }
    }

    pub fn select_next(&mut self) {
        self.step(true);
    }

    pub fn select_prev(&mut self) {
        self.step(false);
    }

    pub fn selected(&self) -> Option<&Track> {
        self.list_state.selected().and_then(|i| self.tracks.get(i))
    }
}

impl super::navigation::ListSelection for RelatedState {
    fn list_len(&self) -> usize {
        self.tracks.len()
    }

    fn cursor(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn set_cursor(&mut self, index: Option<usize>) {
        self.list_state.select(index);
    }
}

/// Renderiza la vista Related: letras (arriba) + lista de recomendados (abajo).
///
/// `position` es el reloj del karaoke y `finished` si la reproducción ya
/// terminó de verdad (estado `Stopped`): al terminar se limpia el panel de
/// letras. `palette` son los tres colores dominantes de la portada, usados
/// para colorear los tres estados de las letras. `palette` es `None` si la
/// miniatura aún no está lista.
#[allow(clippy::too_many_arguments)] // posición, fin y paleta son datos del render
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut RelatedState,
    position: Option<Duration>,
    finished: bool,
    palette: Option<[[u8; 3]; 3]>,
    visual: &crate::visualization::VisualState,
    mouse: &Option<(u16, u16)>,
    click: &mut bool,
    stats: &std::collections::HashMap<String, TrackListeningStats>,
) {
    let has_lyrics = state.synced.as_ref().filter(|s| !s.is_empty()).is_some();
    // El visual necesita al menos un poco de altura; si el terminal es bajo se
    // prescinde de la banda superior y todo el espacio va a la lista.
    let can_visual = area.height >= 24;
    // La banda superior se reserva si hay letras (karaoke), el visual puede
    // ocupar su lugar, o hay que avisar de que las letras no están disponibles.
    let reserve_band = has_lyrics || can_visual || state.synced_unavailable;

    let mut chunks = vec![];
    if reserve_band {
        let band_height = area.height.saturating_sub(7).clamp(6, 18);
        chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(band_height), Constraint::Min(0)])
            .split(area)
            .to_vec();
    } else {
        chunks = vec![area];
    }

    if reserve_band {
        let top_area = chunks[0];
        if can_visual && !has_lyrics {
            // Sin letras en un terminal alto: el visual de la canción ocupa el
            // espacio de la banda (y sus barras usan la paleta de la portada).
            visualizer::render(
                frame,
                top_area,
                visual,
                position.unwrap_or(Duration::ZERO).as_secs_f32(),
                palette,
            );
        } else {
            render_lyrics(frame, top_area, state, position, finished, palette);
        }
    }

    let tracks_area = chunks.last().copied().unwrap_or(area);
    render_tracks_list(
        frame,
        tracks_area,
        state,
        " Recomendaciones ".to_string(),
        "Sin recomendaciones todavía. Pulsa Enter en esta vista para pedirlas a YouTube.",
        mouse,
        click,
        stats,
    );
}

/// Renderiza las letras: modo karaoke si hay LRC (línea activa resaltada y
/// desplazamiento en cascada sobre el buffer circular); si no hay letra
/// sincronizada se muestra un estado limpio — nunca la letra plana antigua.
#[allow(clippy::too_many_arguments)] // posición, fin y paleta son datos del render
fn render_lyrics(
    frame: &mut Frame,
    area: Rect,
    state: &mut RelatedState,
    position: Option<Duration>,
    finished: bool,
    palette: Option<[[u8; 3]; 3]>,
) {
    if state.synced.as_ref().filter(|s| !s.is_empty()).is_some() {
        render_karaoke(frame, area, state, position, finished, palette);
        return;
    }
    if state.synced_unavailable {
        let block = Block::default().borders(Borders::ALL).title(" Letras ");
        let text = Paragraph::new(Line::from(Span::styled(
            "Letras sincronizadas no disponibles",
            Style::new().fg(Color::DarkGray),
        )))
        .block(block)
        .alignment(Alignment::Center);
        frame.render_widget(text, area);
    }
}

/// Prepara el estado del karaoke (línea activa, fin de letra, paleta) y delega
/// el render en la ventana deslizante ([`KaraokeScroller`]).
///
/// Los tres estados (ya leído / en lectura / no leído) adoptan los tres colores
/// dominantes de la portada. El panel se limpia cuando la reproducción terminó
/// de verdad (`finished` = estado `Stopped` del motor): al acabar la canción se
/// vuelve al estado inicial, sin esperar a que el LRC se quede sin versos. Hasta
/// entonces, aunque la última línea ya se haya cantado (outro largo), la línea
/// final permanece visible y el karaoke no "se queda atascado" ni se limpia
/// antes de tiempo.
#[allow(clippy::too_many_arguments)] // posición, fin y paleta son datos del render
fn render_karaoke(
    frame: &mut Frame,
    area: Rect,
    state: &mut RelatedState,
    position: Option<Duration>,
    finished: bool,
    palette: Option<[[u8; 3]; 3]>,
) {
    let Some(sync) = state.synced.as_ref().filter(|s| !s.is_empty()) else {
        return;
    };
    let pos = position.unwrap_or(Duration::ZERO);

    // Fin real de la reproducción: sin línea activa, se limpia todo.
    let active = if finished {
        None
    } else {
        sync.active_index(pos)
    };

    let colors = palette_colors(palette);
    super::widgets::karaoke::render_karaoke(
        frame,
        area,
        &mut state.scroll,
        sync,
        active,
        finished,
        colors,
    );
}

/// Mapea los tres estados del karaoke a los tres colores dominantes de la
/// portada: `(ya leído, en lectura, no leído)`. La línea activa usa el color
/// más dominante; sin paleta cae al esquema fijo.
fn palette_colors(palette: Option<[[u8; 3]; 3]>) -> (Color, Color, Color) {
    match palette {
        Some(p) => (
            Color::Rgb(p[1][0], p[1][1], p[1][2]),
            Color::Rgb(p[0][0], p[0][1], p[0][2]),
            Color::Rgb(p[2][0], p[2][1], p[2][2]),
        ),
        None => (Color::DarkGray, Color::White, Color::Yellow),
    }
}

/// Renderiza la lista de recomendaciones (o un aviso si está vacía).
///
/// La comparten la vista Related (lista completa) y el panel de la vista
/// Now Playing. Acepta ratón para seleccionar la fila bajo el cursor; el
/// scroll de la lista lo gestiona `ListState` automáticamente.
#[allow(clippy::too_many_arguments)] // ratón/estadísticas son datos del render
pub fn render_tracks_list(
    frame: &mut Frame,
    area: Rect,
    state: &mut RelatedState,
    title: String,
    empty: &str,
    mouse: &Option<(u16, u16)>,
    click: &mut bool,
    stats: &std::collections::HashMap<String, TrackListeningStats>,
) {
    if state.tracks.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(empty))
                .block(Block::default().borders(Borders::ALL).title(title)),
            area,
        );
        return;
    }

    let hovered = super::widgets::hover_index(*mouse, area);
    if *click {
        if let Some(i) = hovered {
            state.list_state.select(Some(i));
        }
        *click = false;
    }
    let items: Vec<ListItem> = state
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let artist = t.primary_artist_name().unwrap_or("Desconocido");
            let duration = t
                .duration
                .map(super::widgets::format_duration)
                .unwrap_or_else(|| "duración pendiente".to_string());
            let listened = stats.get(&t.identifier());
            let line = Line::from(vec![
                Span::styled(
                    format!("[{}] ", t.source.label()),
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw(format!("{artist} - {}", t.title)),
                Span::styled(format!("  ({duration})"), Style::new().fg(Color::DarkGray)),
                listened
                    .map(|s| {
                        Span::styled(
                            if s.recently_played {
                                format!("  ● reciente · {}×", s.play_count)
                            } else {
                                format!("  ↻ {}×", s.play_count)
                            },
                            Style::new().fg(Color::Green),
                        )
                    })
                    .unwrap_or_else(|| Span::raw("")),
            ]);
            let style = if Some(i) == hovered {
                Style::new().fg(Color::Gray)
            } else {
                Style::new()
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::new()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut state.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn render_state(
        synced: Option<SyncLyrics>,
        position: Duration,
        finished: bool,
        palette: Option<[[u8; 3]; 3]>,
    ) -> ratatui::buffer::Buffer {
        let mut state = RelatedState {
            synced,
            ..RelatedState::default()
        };
        let backend = TestBackend::new(60, 16);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut state,
                    Some(position),
                    finished,
                    palette,
                    &crate::visualization::VisualState::inactive(),
                    &None,
                    &mut false,
                    &std::collections::HashMap::new(),
                )
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        buf.content().iter().map(|c| c.symbol()).collect::<String>()
    }

    #[test]
    fn last_line_stays_visible_during_outro_and_clears_at_real_end() {
        let sync = SyncLyrics::parse("[00:05] uno\n[00:10] dos\n");

        // En mitad de la canción la línea activa se muestra.
        let mid = render_state(Some(sync.clone()), Duration::from_secs(7), false, None);
        assert!(buffer_text(&mid).contains("dos"), "línea activa visible");

        // Superada la última marca, durante el outro la última línea se queda
        // (el karaoke no se limpia antes del fin real).
        let outro = render_state(Some(sync.clone()), Duration::from_secs(11), false, None);
        assert!(
            buffer_text(&outro).contains("dos"),
            "durante el outro la última línea sigue visible"
        );

        // Al terminar la reproducción (fin real) sí se limpia todo.
        let done = render_state(Some(sync), Duration::from_secs(11), true, None);
        let text = buffer_text(&done);
        assert!(
            !text.contains("dos") && !text.contains("uno"),
            "al fin real se limpia el panel"
        );
    }

    #[test]
    fn outro_does_not_clear_before_real_end() {
        // La canción dura 180s pero el LRC termina a los 10s (outro largo):
        // la última línea sigue visible mientras suena el outro.
        let sync = SyncLyrics::parse("[00:05] uno\n[00:10] dos\n");
        let buf = render_state(Some(sync.clone()), Duration::from_secs(60), false, None);
        assert!(
            buffer_text(&buf).contains("dos"),
            "durante el outro la última línea se queda (no se limpia a los 10s)"
        );

        // Durante la última línea aún se ve.
        let singing = render_state(Some(sync), Duration::from_secs(10), false, None);
        assert!(
            buffer_text(&singing).contains("dos"),
            "durante la última línea aún se ve"
        );
    }

    #[test]
    fn karaoke_clears_at_real_end_when_lrc_lasts_longer() {
        // El LRC marca la última línea a los 200s pero la canción acaba a los
        // 180s: al llegar al fin real sí se limpia (no se queda colgado).
        let sync = SyncLyrics::parse("[00:05] uno\n[03:20] dos\n");
        let buf = render_state(Some(sync), Duration::from_secs(180), true, None);
        let text = buffer_text(&buf);
        assert!(
            !text.contains("dos"),
            "al terminar la canción se limpia aunque el LRC no haya acabado"
        );
    }

    #[test]
    fn karaoke_uses_cover_palette() {
        let sync = SyncLyrics::parse("[00:05] uno\n[00:10] dos\n[00:15] tres\n");
        // En posición 12s: "uno" (ya leído), "dos" (en lectura), "tres" (no leído).
        let buf = render_state(
            Some(sync),
            Duration::from_secs(12),
            false,
            Some([[255, 0, 0], [0, 255, 0], [0, 0, 255]]),
        );
        let cells: Vec<(&str, Color)> = buf.content().iter().map(|c| (c.symbol(), c.fg)).collect();
        assert!(
            cells
                .iter()
                .any(|(s, fg)| *s == "d" && *fg == Color::Rgb(255, 0, 0)),
            "en lectura = color dominante"
        );
        assert!(
            cells
                .iter()
                .any(|(s, fg)| *s == "u" && *fg == Color::Rgb(0, 255, 0)),
            "ya leído = segundo color"
        );
        assert!(
            cells
                .iter()
                .any(|(s, fg)| *s == "t" && *fg == Color::Rgb(0, 0, 255)),
            "no leído = tercer color"
        );
    }

    #[test]
    fn unavailable_lyrics_show_clean_state_without_plain_fallback() {
        // LRCLIB no devolvió `syncedLyrics`: debe verse el estado limpio y
        // jamás la letra plana amarilla de la implementación antigua.
        let mut state = RelatedState::default();
        state.set_synced(None);

        let backend = TestBackend::new(60, 16);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut state,
                    Some(Duration::from_secs(5)),
                    false,
                    None,
                    &crate::visualization::VisualState::inactive(),
                    &None,
                    &mut false,
                    &std::collections::HashMap::new(),
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.contains("Letras sincronizadas no disponibles"),
            "estado limpio de no disponible"
        );
        assert!(
            !buf.content().iter().any(|c| c.fg == Color::Yellow),
            "la letra plana amarilla antigua no debe aparecer"
        );
    }

    #[test]
    fn palette_colors_falls_back_without_palette() {
        assert_eq!(
            palette_colors(None),
            (Color::DarkGray, Color::White, Color::Yellow)
        );
    }
}
