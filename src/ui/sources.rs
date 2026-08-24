//! Vista de fuentes: proveedores activos.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::domain::source::Source;

use crate::ui::widgets::source_panel;

pub fn render(frame: &mut Frame, area: Rect, sources: &[Source]) {
    source_panel::render(frame, area, sources);
}
