//! Estado de la TUI y loop principal de eventos.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::analysis::AudioFeatures;
use crate::app::audio::{PlaybackState, PlaybackStatus};
use crate::app::thumbnail::ThumbnailState;
use crate::domain::source::Source;
use crate::domain::track::Track;
use crate::infrastructure::storage::TrackListeningStats;
use crate::infrastructure::storage::{HistoryEntry, PlaylistRow};
use crate::playback::clamp_seek_target;
use crate::recommendation::RecommendationSession;
use crate::visualization::{ParameterMapper, VisualEngine, VisualPalette};

use super::backend::BackendCommand;
use super::event::{BackendEvent, UiEvent};
use super::navigation::ListSelection;
use super::search::SearchState;
use super::settings::SettingsForm;
use super::view::View;
use super::VisualContent;
use super::{dashboard, history, metadata, related, search, settings, sources};

/// Un aviso de diagnóstico transitorio para el pie de página.
#[derive(Debug)]
struct Notice {
    text: String,
    expires: std::time::Instant,
}

/// Pie de diagnóstico discreto: un anillo acotado de avisos (errores de stream,
/// overrun, cortes) que la UI muestra abajo a la derecha, con caducidad, para
/// no ensuciar la línea de estado ni el contenido de las vistas.
#[derive(Debug, Default)]
struct Notices {
    items: std::collections::VecDeque<Notice>,
}

impl Notices {
    /// Avisos simultáneos que caben en el pie.
    const CAP: usize = 3;
    /// Cuánto permanece un aviso visible tras el último reporte.
    const LIFETIME: std::time::Duration = std::time::Duration::from_secs(8);

    /// Registra un aviso. Repetir el mismo texto mientras sigue vivo renueva
    /// su caducidad (rate-limit: un overrun que se repite no llena la pantalla).
    fn push(&mut self, text: String, now: std::time::Instant) {
        if let Some(last) = self.items.back_mut() {
            if last.text == text {
                last.expires = now + Self::LIFETIME;
                return;
            }
        }
        self.items.push_back(Notice {
            text,
            expires: now + Self::LIFETIME,
        });
        while self.items.len() > Self::CAP {
            self.items.pop_front();
        }
    }

    /// Texto de los avisos vigentes ahora mismo (unidos), vacío si no hay.
    fn active(&self, now: std::time::Instant) -> String {
        self.items
            .iter()
            .filter(|n| n.expires > now)
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

pub struct App {
    view: View,
    should_quit: bool,
    backend_tx: UnboundedSender<BackendCommand>,
    search: SearchState,
    now_playing: Option<Track>,
    playback: PlaybackStatus,
    related: related::RelatedState,
    history: Vec<HistoryEntry>,
    /// Recencia/frecuencia por `Track::identifier()`, cargada desde SQLite.
    listening_stats: std::collections::HashMap<String, TrackListeningStats>,
    sources: Vec<Source>,
    settings: Option<SettingsForm>,
    playlists: Vec<PlaylistRow>,
    // Estado del ratón: posición (columna, fila) del último evento y si ha
    // habido un click izquierdo pendiente. El render de una vista con lista
    // (Search, Related, Now Playing) lo consume para seleccionar la fila bajo
    // el cursor.
    mouse_pos: Option<(u16, u16)>,
    mouse_click: bool,
    status: Option<String>,
    /// Pie de diagnóstico discreto: errores de stream en caliente (overrun,
    /// cortes, decodificación) mostrados abajo a la derecha con caducidad, sin
    /// escribir en la interfaz ni romperla.
    notices: Notices,
    /// Reproducción automática de recomendaciones (conmutable con `a`).
    autoplay: bool,
    /// Sesión de recomendaciones (spec §13/§26): qué track tiene cargadas sus
    /// recomendaciones, cuál está en vuelo y con qué generación. Evita
    /// re-pedir lo mismo en cada tick/redibujado y descarta respuestas de
    /// sesiones anteriores (incluso si repiten la misma canción).
    recs: RecommendationSession,
    /// Canción que el usuario acaba de pedir. Mientras el backend prepara su
    /// stream, los ticks y respuestas de la canción anterior no son válidos
    /// para la UI ni para las letras.
    pending_track: Option<String>,
    /// Miniatura de cada track pedido al backend, clave = identificador
    /// estable. El estado incluye la imagen decodificada cuando está lista.
    thumbnails: std::collections::HashMap<String, ThumbnailState>,
    /// Motor visual (mapper + inercia) alimentado por features+posición.
    visual: VisualEngine,
    /// Contenido elegido para la banda superior de Related (spec §16/§17).
    visual_mode: VisualContent,
    /// Último snapshot de features recibido del backend.
    features: Option<Arc<AudioFeatures>>,
    /// Instante de recepción del último snapshot (frescura ~900 ms).
    features_at: Option<std::time::Instant>,
    /// Contador de frames para animaciones de la TUI (spinner, avisos).
    frame: u64,
    /// Reloj maestro de posición (audio = fuente de verdad, spec §17).
    /// Consumidores: karaoke/letras, progreso y futura visualización.
    /// Monótono por track, extrapolado entre muestras del motor, con seek
    /// pendiente hasta confirmación. Lógica extraída a `playback::PositionClock`.
    clock: crate::playback::PositionClock,
}

impl App {
    pub fn new(backend_tx: UnboundedSender<BackendCommand>) -> Self {
        Self {
            view: View::NowPlaying,
            should_quit: false,
            backend_tx,
            search: SearchState::default(),
            now_playing: None,
            playback: PlaybackStatus {
                track: None,
                state: PlaybackState::Stopped,
                position: std::time::Duration::ZERO,
                duration: None,
                stalled: false,
            },
            related: related::RelatedState::default(),
            history: Vec::new(),
            listening_stats: std::collections::HashMap::new(),
            sources: Vec::new(),
            settings: None,
            playlists: Vec::new(),
            mouse_pos: None,
            mouse_click: false,
            status: None,
            notices: Notices::default(),
            autoplay: true,
            recs: RecommendationSession::new(),
            pending_track: None,
            thumbnails: std::collections::HashMap::new(),
            visual: VisualEngine::new(ParameterMapper::default()),
            visual_mode: VisualContent::default(),
            features: None,
            features_at: None,
            frame: 0,
            clock: crate::playback::PositionClock::new(),
        }
    }

    pub async fn run<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        mut ui_rx: UnboundedReceiver<UiEvent>,
        mut backend_rx: UnboundedReceiver<BackendEvent>,
    ) -> anyhow::Result<()> {
        self.request_initial_data();
        self.draw(terminal)?;

        // Redibujado periódico SOLO mientras hay animación: spinner de
        // buffering/stream lento, barra de progreso y extrapolación del
        // karaoke. Sin animación la UI se redibuja únicamente al recibir
        // eventos, de modo que una terminal en reposo no quema ciclos.
        let mut redraw = tokio::time::interval(std::time::Duration::from_millis(250));
        redraw.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let animated = self.animation_active();
            tokio::select! {
                _ = redraw.tick(), if animated => {
                    self.draw(terminal)?;
                }
                ui = ui_rx.recv() => {
                    match ui {
                        Some(UiEvent::Key(key)) => self.on_key(key),
                        Some(UiEvent::Mouse(mouse)) => self.on_mouse(mouse),
                        Some(UiEvent::Resize(..)) => self.mouse_pos = None,
                        None => self.should_quit = true,
                    }
                    self.draw(terminal)?;
                }
                be = backend_rx.recv() => {
                    if let Some(event) = be {
                        self.on_backend(event);
                        self.draw(terminal)?;
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    /// Redibuja un frame (avanza el contador de animación).
    fn draw<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> anyhow::Result<()> {
        self.frame = self.frame.wrapping_add(1);
        terminal.draw(|f| self.render(f))?;
        Ok(())
    }

    /// `true` si la vista actual tiene algo en movimiento (spinner de
    /// buffering, barra de progreso, karaoke): entonces el redibujado
    /// periódico está activo.
    fn animation_active(&self) -> bool {
        matches!(
            self.playback.state,
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
        )
    }

    fn request_initial_data(&self) {
        let _ = self.backend_tx.send(BackendCommand::LoadHistory);
        let _ = self.backend_tx.send(BackendCommand::LoadListeningStats);
        let _ = self.backend_tx.send(BackendCommand::LoadSources);
        let _ = self.backend_tx.send(BackendCommand::LoadSettings);
        let _ = self.backend_tx.send(BackendCommand::ListPlaylists);
        let _ = self
            .backend_tx
            .send(BackendCommand::SetAutoplay(self.autoplay));
    }

    // ------------------------------------------------------- entrada

    fn on_mouse(&mut self, mouse: MouseEvent) {
        self.mouse_pos = Some((mouse.column, mouse.row));
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.mouse_click = true;
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        // Ctrl+C sale siempre (incluso editando).
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        // Navegación global Shift+1..Shift+7. `from_shift_key` acepta tanto el
        // dígito con modificador SHIFT (protocolo kitty) como el símbolo que
        // escribe Shift+dígito en la mayoría de terminales (`!@#$%^&`), de modo
        // que nunca choca con un dígito suelto ni con Ctrl+dígito.
        if let Some(view) = View::from_shift_key(key.code, key.modifiers) {
            self.switch_view(view);
            return;
        }

        match self.view {
            View::Search => self.on_search_key(key),
            View::Settings => self.on_settings_key(key),
            _ => self.on_read_only_key(key),
        }
    }

    /// Teclas en vistas de solo lectura (Now Playing, Related, Sources, ...).
    ///
    /// Los dígitos sueltos **no** cambian de vista: solo los atajos
    /// `Shift+1..7` navegan. Así `3` nunca abre búsqueda por accidente y
    /// `Shift+3` (el símbolo `#`) siempre es Search en cualquier terminal.
    fn on_read_only_key(&mut self, key: KeyEvent) {
        match key.code {
            // Salir solo se acepta aquí: si hay texto en edición el evento ya se
            // redirige a on_search_key/on_settings_key antes de llegar.
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Esc => self.switch_view(View::NowPlaying),
            KeyCode::Char(' ') => {
                let _ = self.backend_tx.send(BackendCommand::Toggle);
            }
            KeyCode::Char('a') => {
                self.autoplay = !self.autoplay;
                let _ = self
                    .backend_tx
                    .send(BackendCommand::SetAutoplay(self.autoplay));
                self.status = Some(if self.autoplay {
                    "Autoplay activado: al terminar se reproduce la siguiente recomendación."
                        .to_string()
                } else {
                    "Autoplay desactivado.".to_string()
                });
            }
            // Contenido de la banda superior (spec §16): Auto → letras si las
            // hay, si no el visualizador; Letras y Visual fuerzan su modo. El
            // cambio es puro de presentación: no regenera recomendaciones, no
            // toca la reproducción y no destruye el estado del otro modo.
            KeyCode::Char('v') => {
                self.visual_mode = self.visual_mode.next();
                self.status = Some(format!(
                    "Contenido de la banda: {} (Auto: letras si las hay, si no el visual)",
                    self.visual_mode.label()
                ));
            }
            // Salto entre recomendaciones: Shift+D avanza a la siguiente de la
            // cola y Shift+A vuelve a la anterior. Llegan como mayúscula (con o
            // sin SHIFT según el protocolo del terminal), distinta de la `a`
            // minúscula que alterna el autoplay.
            KeyCode::Char('D') => self.skip_track(true),
            KeyCode::Char('A') => self.skip_track(false),
            KeyCode::Left | KeyCode::Right => {
                let delta = if key.code == KeyCode::Left {
                    -10i64
                } else {
                    10
                };
                // Parte del reloj de karaoke (más reciente que el ticker del
                // motor) y limita ambos extremos con `clamp_seek_target`:
                // retroceder desde 5s diez segundos siempre produce exactamente
                // 0, nunca un underflow, y nunca supera la duración conocida.
                let target = clamp_seek_target(self.karaoke_now(), delta, self.playback.duration);
                // El reloj del karaoke queda pendiente de resincronizar con el
                // motor, pero NO salta al objetivo de inmediato: el motor
                // pre-descarga la región del salto y la canción sigue sonando
                // desde la posición actual hasta que el salto se ejecuta, así
                // que el karaoke debe seguir ese reloj real (lo re-ancla
                // `update_karaoke_clock` con cada muestra).
                self.clock.begin_seek(target);
                // Guarda de sesión (FASE 4): etiquetamos el seek con el track
                // en curso. Si al ejecutarse (la orden es asíncrona y puede
                // bloquear descargando la región del salto) el usuario ya
                // reprodujo otra canción, el backend lo descarta para no
                // saltar dentro de la canción nueva.
                let for_track = self
                    .now_playing
                    .as_ref()
                    .map(|t| t.identifier())
                    .unwrap_or_default();
                let _ = self
                    .backend_tx
                    .send(BackendCommand::Seek(target.as_secs(), for_track));
            }
            // Navegación de listas (spec §14): `W`/`S` y `↑`/`↓` mueven la
            // selección de la vista Related y del panel de la Now Playing con
            // la misma lógica (`ListSelection`). En la vista Search, `W`/`S`
            // siguen escribiendo en la consulta (solo `↑`/`↓` navegan).
            KeyCode::Char('w') | KeyCode::Up => {
                if self.view == View::Related || self.view == View::NowPlaying {
                    self.related.step(false);
                }
            }
            KeyCode::Char('s') | KeyCode::Down => {
                if self.view == View::Related || self.view == View::NowPlaying {
                    self.related.step(true);
                }
            }
            // Enter guarda y reproduce la selección, o recarga recomendaciones
            // si la lista está vacía (regeneración EXPLÍCITA del usuario, la
            // única que está permitida además de la carga por cambio de canción).
            KeyCode::Enter => {
                let has_list = self.view == View::Related || self.view == View::NowPlaying;
                if has_list {
                    if let Some(track) = self.related.selected().cloned() {
                        self.save_and_play(track);
                    } else {
                        self.reload_related();
                    }
                }
            }
            _ => {}
        }
    }

    fn on_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.switch_view(View::NowPlaying);
            }
            KeyCode::Enter => {
                if self.search.editing || self.search.results.is_empty() {
                    let query = self.search.text().trim().to_string();
                    if query.is_empty() {
                        return;
                    }
                    self.search.last_query = Some(query.clone());
                    self.search.editing = false;
                    self.search.searching = true;
                    self.search.results.clear();
                    self.search.related_from = 0;
                    self.search.list_state.select(None);
                    self.status = Some(format!("Buscando «{query}»..."));
                    let _ = self.backend_tx.send(BackendCommand::Search(query));
                } else if let Some(track) = self.search.selected().cloned() {
                    self.save_and_play(track);
                }
            }
            KeyCode::Up => self.search.select_prev(),
            KeyCode::Down => self.search.select_next(),
            KeyCode::Left => self.search.move_cursor_left(),
            KeyCode::Right => self.search.move_cursor_right(),
            KeyCode::Home => self.search.move_cursor_home(),
            KeyCode::End => self.search.move_cursor_end(),
            KeyCode::Backspace => self.search.backspace(),
            KeyCode::Delete => self.search.delete(),
            KeyCode::Char('\n') => {}
            KeyCode::Char(c) => self.search.insert_char(c),
            _ => {}
        }
    }

    fn on_settings_key(&mut self, key: KeyEvent) {
        if let Some(settings) = self.settings.as_mut() {
            match key.code {
                KeyCode::Esc => self.switch_view(View::NowPlaying),
                KeyCode::Enter => {
                    let form = settings.form.clone();
                    self.status = Some("Guardando ajustes en .env...".to_string());
                    let _ = self
                        .backend_tx
                        .send(BackendCommand::SaveSettings(Box::new(form)));
                }
                KeyCode::Up => settings.move_focus(-1),
                KeyCode::Down | KeyCode::Tab => settings.move_focus(1),
                KeyCode::Backspace => settings.backspace(),
                KeyCode::Char(c) => settings.insert_char(c),
                _ => {}
            }
        }
    }

    /// Pide recomendaciones + letra del track en curso al backend.
    ///
    /// La SESIÓN decide si hace falta pedir (spec §13): devuelve `None` (no
    /// envía nada) si ya están cargadas para esta canción o ya hay una petición
    /// en vuelo. Cambiar de vista, redibujar o un tick de reproducción nunca
    /// pueden disparar una recarga por aquí.
    fn load_related(&mut self) {
        let Some(track) = self.now_playing.clone() else {
            self.status = Some("Sin canción seleccionada. Reproduce algo primero.".to_string());
            return;
        };
        let id = track.identifier();
        let Some(generation) = self.recs.request(&id) else {
            return;
        };
        self.status = Some(format!("Cargando recomendaciones de «{}»...", track.title));
        let _ = self
            .backend_tx
            .send(BackendCommand::LoadRelated(Box::new(track), generation));
    }

    /// Recarga de recomendaciones EXPLÍCITA (Enter sobre una lista vacía).
    /// A diferencia de `load_related`, descarta la sesión actual (cargada o en
    /// vuelo) para que la nueva petición arranque con generación fresca.
    fn reload_related(&mut self) {
        self.recs.reset();
        self.related.tracks.clear();
        self.related.list_state.select(None);
        self.related.clear_lyrics();
        self.load_related();
    }

    /// Actualiza `now_playing` y dispara la carga de recomendaciones si el
    /// track cambió (o aún no hay ninguna cargada). Los guardas de la sesión
    /// evitan re-pedir las mismas recomendaciones en cada tick de reproducción.
    fn on_new_now_playing(&mut self, track: Track) {
        self.request_thumbnail(&track);
        let id = track.identifier();
        let changed = self.now_playing.as_ref().map(|n| n.identifier()) != Some(id.clone());
        self.now_playing = Some(track);
        if changed {
            // Las recomendaciones son propiedad de una canción concreta. No
            // dejes que autoplay use la cola anterior mientras llega la nueva
            // ni que una respuesta tardía pueda aparentar vigencia.
            self.related.tracks.clear();
            self.related.list_state.select(None);
            self.related.clear_lyrics();
            self.recs.on_track_changed();
        }
        // La sesión decide: sin cambio de canción y con recomendaciones ya
        // cargadas/en vuelo para esta canción, `load_related` no envía nada.
        self.load_related();
    }

    /// Pide la miniatura de un track al backend si aún no se resolvió (o no
    /// está en curso). La UI marca `Loading` para que la tarjeta lo refleje.
    fn request_thumbnail(&mut self, track: &Track) {
        let key = track.identifier();
        if self.thumbnails.contains_key(&key) {
            return;
        }
        self.thumbnails.insert(key, ThumbnailState::Loading);
        let _ = self
            .backend_tx
            .send(BackendCommand::Thumbnail(Box::new(track.clone())));
    }

    /// Guarda el track en la BD y lo reproduce (stream resuelto por el backend).
    fn save_and_play(&mut self, track: Track) {
        self.status = Some(format!("Guardando «{}»...", track.title));
        self.pending_track = Some(track.identifier());
        // La intención del usuario pasa a ser el estado autoritativo ya. Así
        // una respuesta LRC/tick de la canción anterior se descarta durante el
        // buffering en vez de volver a pintar sus letras sobre la nueva.
        self.on_new_now_playing(track.clone());
        // Feedback inmediato: la canción nueva tarda en resolverse/descargarse
        // y el backend confirma con un evento cuando arranca, así que la UI se
        // adelanta mostrando "preparando" en vez de la canción anterior.
        self.playback.state = PlaybackState::Buffering;
        self.playback.stalled = false;
        // Nueva canción: el reloj parte de cero y se descartan las letras de
        // la anterior (y la plantilla del karaoke) hasta que lleguen las del
        // track nuevo. Sin esto, durante el buffering se vería la canción
        // anterior en el panel de letras.
        self.clock.clear();
        self.related.clear_lyrics();
        let _ = self.backend_tx.send(BackendCommand::Play(Box::new(track)));
    }

    /// Salta a la siguiente/anterior recomendación de la cola y la reproduce.
    ///
    /// El backend es la fuente de verdad de la cola (se mantiene estable entre
    /// canciones), así que el salto se delega en él: `Shift+D` y `Shift+A`
    /// funcionan aunque la lista visual de la UI aún esté vacía durante el
    /// buffering de la canción nueva. El backend responde con la reproducción
    /// o un aviso en la línea de estado si no hay cola.
    fn skip_track(&mut self, forward: bool) {
        let cmd = if forward {
            BackendCommand::NextTrack
        } else {
            BackendCommand::PrevTrack
        };
        let _ = self.backend_tx.send(cmd);
    }

    fn switch_view(&mut self, view: View) {
        self.view = view;
        match view {
            View::History => {
                let _ = self.backend_tx.send(BackendCommand::LoadHistory);
            }
            View::Sources => {
                let _ = self.backend_tx.send(BackendCommand::LoadSources);
            }
            View::Settings => {
                let _ = self.backend_tx.send(BackendCommand::LoadSettings);
            }
            // Cambiar a Related NO pide recomendaciones (spec §7/§18): solo
            // muestra las de la sesión actual (cargadas al empezar la canción).
            // La regeneración explícita es Enter sobre una lista vacía.
            View::Metadata | View::NowPlaying | View::Search | View::Related => {}
        }
    }

    fn on_backend(&mut self, event: BackendEvent) {
        match event {
            BackendEvent::SearchResults { outcome, .. } => {
                self.search.searching = false;
                let outcome = *outcome;
                let count = outcome.items.len();
                let errors = outcome.errors.len();
                self.search.related_from = outcome.related_from;
                self.search.results = outcome.items;
                if !self.search.results.is_empty() {
                    self.search.list_state.select(Some(0));
                }
                self.status = Some(if count == 0 {
                    format!("Sin resultados ({errors} error(es). ¿Red sobre YouTube?")
                } else if self.search.related_from > 0 {
                    format!(
                        "{} resultado(s) + {} relacionadas. Enter guarda el seleccionado.",
                        self.search.related_from,
                        count - self.search.related_from
                    )
                } else {
                    format!("{count} resultado(s). Enter guarda el seleccionado.")
                });
            }
            BackendEvent::TrackSaved { track, internal_id } => {
                let track = *track;
                self.status = Some(format!(
                    "Guardado «{}» en BD (track_id={internal_id}).",
                    track.title
                ));
            }
            BackendEvent::History(entries) => self.history = entries,
            BackendEvent::ListeningStats(stats) => {
                self.listening_stats = stats.into_iter().map(|s| (s.key.clone(), s)).collect();
            }
            BackendEvent::Sources(sources) => self.sources = sources,
            BackendEvent::Settings(form) => {
                self.settings = Some(SettingsForm::new(form));
                self.status = Some("Ajustes · Enter guarda en .env, Esc vuelve.".to_string());
            }
            BackendEvent::Playback(mut status) => {
                // Solo se aceptan ticks del track pedido (pending) o del que la
                // UI ya muestra: los ticks obsoletos (p. ej. la respuesta de un
                // seek emitida mientras otra canción arrancaba) no deben volver
                // a pintar la anterior ni recuperar sus letras.
                let stale = status.track.as_ref().is_some_and(|t| {
                    self.pending_track.as_deref() != Some(t.identifier().as_str())
                        && self.now_playing.as_ref().map(|n| n.identifier()) != Some(t.identifier())
                });
                if stale {
                    return;
                }
                // Los ticks del motor llevan duración en el snapshot; copiarla
                // al track evita que una actualización posterior vuelva a
                // mostrar `-` en la tarjeta o metadatos.
                if let Some(track) = status.track.as_mut().filter(|t| t.duration.is_none()) {
                    track.duration = status.duration;
                }
                if let Some(track) = status.track.clone() {
                    if self.pending_track.as_deref() == Some(track.identifier().as_str()) {
                        self.pending_track = None;
                    }
                    self.on_new_now_playing(track);
                }
                self.update_karaoke_clock(&status);
                self.playback = status;
                if self.view == View::NowPlaying && !self.recs.is_loading() {
                    self.status = Some(self.playback_line());
                }
            }
            BackendEvent::PlaybackStarted { mut status, stats } => {
                // Un arranque de OTRA canción (p. ej. la tarea de autoplay que
                // ganó la carrera contra el Play del usuario) no debe pisar la
                // que el usuario pidió mientras sigue en marcha.
                if self.pending_track.as_ref().is_some_and(|pending| {
                    status
                        .track
                        .as_ref()
                        .is_none_or(|track| track.identifier() != *pending)
                }) {
                    return;
                }
                if let Some(track) = status.track.as_mut().filter(|t| t.duration.is_none()) {
                    track.duration = status.duration;
                }
                self.listening_stats = stats.into_iter().map(|s| (s.key.clone(), s)).collect();
                let _ = self.backend_tx.send(BackendCommand::LoadHistory);
                if let Some(track) = status.track.clone() {
                    if self.pending_track.as_deref() == Some(track.identifier().as_str()) {
                        self.pending_track = None;
                    }
                    // Todo arranque confirmado es una canción nueva, también si
                    // es el MISMO track (autoplay que vuelve al inicio de la
                    // cola): el reloj del karaoke y el seek pendiente se
                    // reinician con ella.
                    if self.now_playing.as_ref().map(|n| n.identifier()) == Some(track.identifier())
                    {
                        // Replay del mismo track: la letra sigue siendo válida,
                        // solo se rebobina la ventana y el reloj parte de cero.
                        // (El reloj conserva el track: `update` no lo trataría
                        // como canción nueva ni descartaría la letra.)
                        self.related.scroll.reset();
                        self.clock.restart_same_track();
                    } else {
                        // Canción distinta: el reloj se apaga para que la
                        // siguiente muestra lo trate como track nuevo (y
                        // descarte la letra anterior).
                        self.clock.clear();
                    }
                    self.clock.cancel_pending_seek();
                    self.on_new_now_playing(track);
                }
                self.update_karaoke_clock(&status);
                self.playback = status;
            }
            BackendEvent::PlaybackError(err) => {
                // Si la reproducción pedida falló se abandona el estado
                // optimista ("preparando"): la UI no debe quedar colgada. El
                // siguiente evento del motor (si lo hay) restablecerá lo que
                // realmente suena.
                if self.pending_track.take().is_some() {
                    self.playback.state = PlaybackState::Stopped;
                    self.playback.track = None;
                    self.playback.stalled = false;
                    self.clock.cancel_pending_seek();
                }
                self.status = Some(format!("Reproducción: {err}"));
            }
            // Seek iniciado: estado transitorio "buscando". No movemos el reloj
            // (el audio real sigue donde estaba) ni la UI del karaoke: solo
            // flota el estado para que el usuario vea que el salto está en
            // marcha.
            BackendEvent::SeekStarted => {
                self.playback.state = PlaybackState::Seeking;
            }
            // Seek CONFIRMADO por el backend: el audio real ya está en el
            // objetivo. Re-anclamos el reloj al objetivo elegido y limpiamos el
            // deseo pendiente — sin depender de que una muestra del motor
            // coincida.
            BackendEvent::SeekCompleted => {
                let now = std::time::Instant::now();
                self.clock.confirm_seek(now);
                // Restablece el estado de reproducción real (pausado o
                // reproduciendo) según lo que indique el siguiente tick del
                // motor; si fue un seek mientras se reproducía, queda Playing.
                if self.playback.state != PlaybackState::Paused {
                    self.playback.state = PlaybackState::Playing;
                }
            }
            // Seek FALLIDO: el audio NO cambió. Cancelamos el deseo pendiente
            // para que el reloj siga al audio real que nunca se movió.
            BackendEvent::SeekFailed => {
                self.clock.cancel_pending_seek();
                self.status = Some(
                    "No se pudo buscar la posición (stream no busca hacia atrás).".to_string(),
                );
            }
            // Errores de stream en caliente: no tocan la línea de estado; van
            // al pie de página discreto (abajo a la derecha) con caducidad.
            BackendEvent::StreamError(err) => {
                self.notices.push(err, std::time::Instant::now());
            }
            BackendEvent::Related {
                track,
                related,
                synced,
                generation,
            } => {
                let id = track.identifier();
                // Sesión (spec §13/§26): la respuesta solo vale si pertenece a
                // la carga EN VUELO (mismo track y misma generación). Una
                // respuesta tardía de una sesión anterior — incluso para el
                // MISMO track que ahora suena de nuevo — se descarta sin tocar
                // nada: no puebla la lista ni libera la petición en curso.
                if !self.recs.complete(&id, generation) {
                    return;
                }
                self.related.tracks = related;
                // LRCLIB ya devolvió el mejor resultado posible (o la caché
                // local): `None` significa que no hay LRC para esta canción
                // y se muestra el estado limpio, nunca la letra plana.
                match synced {
                    Some(s) if !s.trim().is_empty() => {
                        let parsed = crate::domain::lyrics::SyncLyrics::parse(&s);
                        if parsed.is_empty() {
                            self.related.set_synced(None);
                        } else {
                            self.related.set_synced(Some(parsed));
                        }
                    }
                    _ => self.related.set_synced(None),
                }
                self.related.list_state.select(Some(0));
                let _ = self.backend_tx.send(BackendCommand::SetAutoplayQueue(
                    self.related.tracks.clone(),
                ));
                self.status = Some(format!(
                    "{} recomendaciones. ↑/↓ o W/S selecciona, Enter reproduce.",
                    self.related.tracks.len()
                ));
            }
            BackendEvent::Thumbnail { key, state } => {
                self.thumbnails.insert(key, state);
            }
            // Nuevo frame de análisis: se guarda; el DIBUJO lo dispara el
            // propio evento (cada Features ⇒ un redraw a ~15 Hz).
            BackendEvent::Features(f) => {
                self.features = Some(f);
                self.features_at = Some(std::time::Instant::now());
            }
            BackendEvent::Playlists(playlists) => self.playlists = playlists,
            BackendEvent::PlaylistTracks { .. } => {}
            BackendEvent::Message(msg) => self.status = Some(msg),
            BackendEvent::Error(err) => {
                self.search.searching = false;
                // El error puede haber dejado una petición de recomendaciones
                // colgada: se aborta para no esperar una respuesta que nunca
                // llegará ni aceptarla si llegara.
                self.recs.abort();
                self.status = Some(format!("Error: {err}"));
            }
        }
    }

    // ------------------------------------------------------- render

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_header(frame, chunks[0]);
        self.render_view(frame, chunks[1]);
        self.render_status(frame, chunks[2]);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let shortcuts = View::ALL
            .iter()
            .map(|v| format!("Shift+{} {}", v.shortcut_digit(), v.label()))
            .collect::<Vec<_>>()
            .join("  ");
        let text = vec![
            Line::styled(
                format!(
                    " PlayFusion — {} · {}",
                    self.view.label(),
                    self.playback_line()
                ),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Line::from(format!("{shortcuts}    ·   1..7 sueltos no navegan")),
        ];
        frame.render_widget(
            Paragraph::new(text).block(Block::default().borders(Borders::BOTTOM)),
            area,
        );
    }

    fn playback_line(&self) -> String {
        let state = match self.playback.state {
            PlaybackState::Playing => "▶ reproduciendo",
            PlaybackState::Paused => "⏸ pausado",
            PlaybackState::Stopped => "⏹ detenido",
            PlaybackState::Buffering => "⏳ preparando",
            PlaybackState::Seeking => "🎚 buscando",
        };
        let track = self
            .playback
            .track
            .as_ref()
            .map(|t| t.display_title())
            .unwrap_or_else(|| "sin track".to_string());
        if self.playback.stalled {
            format!(
                "[{state} {} stream lento · rellenando…] {track}",
                crate::ui::widgets::spinner_phase(self.frame)
            )
        } else {
            format!("[{state}] {track}")
        }
    }

    fn render_view(&mut self, frame: &mut Frame, area: Rect) {
        match self.view {
            View::NowPlaying => {
                // Frescura: sin datos recientes (pausa larga/fin) el visual
                // pasa a inactivo y las barras caen con su inercia.
                let fresh = self
                    .features_at
                    .filter(|t| t.elapsed() < std::time::Duration::from_millis(900))
                    .and_then(|_| self.features.clone());
                let position = self.karaoke_now();
                // La paleta de la portada entra al MOTOR: el engine la funde con
                // la anterior y la escena la expone ya mezclada al renderer.
                let palette = VisualPalette::from_cover(self.cover_palette());
                let state = self.visual.update(fresh.as_ref(), position, &palette);
                dashboard::render(
                    frame,
                    area,
                    &self.playback,
                    &mut self.related,
                    self.autoplay,
                    &self.mouse_pos,
                    &mut self.mouse_click,
                    &self.thumbnails,
                    self.frame,
                    &self.listening_stats,
                    &state,
                );
            }
            View::Related => {
                let position = self.karaoke_now();
                // Mismo cálculo de frescura que Now Playing: si el visual va a
                // ocupar el espacio de las letras (cuando no las hay), su
                // escena refleja el análisis actual (o queda dormida).
                let fresh = self
                    .features_at
                    .filter(|t| t.elapsed() < std::time::Duration::from_millis(900))
                    .and_then(|_| self.features.clone());
                let palette = VisualPalette::from_cover(self.cover_palette());
                let visual = self.visual.update(fresh.as_ref(), position, &palette);
                related::render(
                    frame,
                    area,
                    &mut self.related,
                    Some(position),
                    // Fin real de la reproducción: el karaoke se limpia cuando
                    // el motor está `Stopped` (canción acabada o detenida), no
                    // al superar la última línea del LRC.
                    self.playback.state == PlaybackState::Stopped,
                    self.visual_mode,
                    &visual,
                    &self.mouse_pos,
                    &mut self.mouse_click,
                    &self.listening_stats,
                );
            }
            View::Search => search::render(
                frame,
                area,
                &mut self.search,
                &self.mouse_pos,
                &mut self.mouse_click,
                &self.listening_stats,
            ),
            View::Sources => sources::render(frame, area, &self.sources),
            View::Metadata => metadata::render(frame, area, self.now_playing.as_ref()),
            View::History => history::render(frame, area, &self.history),
            View::Settings => {
                if let Some(settings) = self.settings.as_mut() {
                    settings::render(frame, area, settings);
                }
            }
        }
    }

    /// Paleta de tres colores dominantes de la portada del track en curso, si
    /// la miniatura ya está decodificada. Se usa para colorear el karaoke.
    fn cover_palette(&self) -> Option<[[u8; 3]; 3]> {
        self.now_playing
            .as_ref()
            .and_then(|t| self.thumbnails.get(&t.identifier()))
            .and_then(|state| match state {
                ThumbnailState::Loaded(img) => img.palette,
                _ => None,
            })
    }

    /// Posición "ahora mismo" según el reloj maestro (delega en
    /// [`crate::playback::PositionClock::snapshot`], que extrapola mientras se
    /// reproduce y se congela en pausa/stall).
    fn karaoke_now(&self) -> std::time::Duration {
        self.clock.snapshot(
            self.playback.state == PlaybackState::Playing,
            self.playback.stalled,
            self.playback.duration,
            std::time::Instant::now(),
        )
    }

    /// Incorpora la muestra del motor al reloj maestro y reacciona a sus
    /// eventos (limpieza de letras al cambiar/terminar canción).
    ///
    /// La lógica del reloj vive íntegramente en `playback::PositionClock`;
    /// aquí solo queda el efecto colateral de UI.
    fn update_karaoke_clock(&mut self, status: &PlaybackStatus) {
        let key = status.track.as_ref().map(|t| t.identifier());
        match self
            .clock
            .update(key.as_deref(), status.position, std::time::Instant::now())
        {
            // Canción terminada/detenida: sin reloj activo y sin letras: la
            // canción ya no suena y la ventana del karaoke no debe quedarse
            // pintada con su letra.
            Some(crate::playback::ClockEvent::Cleared) => {
                self.related.clear_lyrics();
            }
            // Nueva canción: descarta la letra de la anterior mientras llegan
            // las del track en curso.
            Some(crate::playback::ClockEvent::NewTrack) => {
                self.related.clear_lyrics();
            }
            None => {}
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let msg = self
            .status
            .as_deref()
            .unwrap_or("q: salir · Shift+1..7: cambiar vista");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("› ", Style::new().fg(Color::Green)),
                Span::raw(msg),
            ]))
            .style(Style::new().fg(Color::Gray)),
            area,
        );

        // Pie de página discreto: los errores de stream en caliente se pintan
        // SOLO sobre la fila de estado, alineados a la derecha y sobre su
        // propio ancho, de modo que ni reordenan la interfaz ni la rompen
        // (nunca se inserta texto en el contenido de las vistas). Caducan solo.
        let notice = self.notices.active(std::time::Instant::now());
        if !notice.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    format!("☈ {notice} "),
                    Style::new().fg(Color::DarkGray),
                )]))
                .alignment(Alignment::Right)
                .style(Style::new().fg(Color::DarkGray)),
                area,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::app::audio::{PlaybackState, PlaybackStatus};
    use crate::domain::{album::Album, artist::Artist, genre::Genre};
    use crate::domain::{source::Source, track::Track};
    use ratatui::backend::TestBackend;
    use tokio::sync::mpsc::unbounded_channel;

    fn sample_track() -> Track {
        let artist = Artist::new("Queen".to_string(), None, None, None);
        let mut track = Track::new(
            "Bohemian Rhapsody".to_string(),
            vec![artist],
            Source::YouTube,
        );
        track.album = Some(Album::new(
            "A Night at the Opera".to_string(),
            None,
            None,
            None,
        ));
        track.genres = vec![Genre::new("rock".to_string())];
        track.duration = Some(std::time::Duration::from_secs(354));
        track
    }

    /// Mismo track base pero con `external_id` distinto (identificables).
    fn rec_track(id: &str) -> Track {
        let mut t = sample_track();
        t.external_id = Some(id.to_string());
        t
    }

    /// Drena la cola del backend; `true` si hubo el comando esperado.
    fn sent_skip(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<BackendCommand>,
        expect: BackendCommand,
    ) -> bool {
        let mut found = false;
        while let Ok(c) = rx.try_recv() {
            if std::mem::discriminant(&c) == std::mem::discriminant(&expect) {
                found = true;
            }
        }
        found
    }

    #[test]
    fn notices_dedup_repeating_stream_errors() {
        use std::time::Duration as StdDuration;
        let mut notices = Notices::default();
        let t0 = std::time::Instant::now();
        notices.push("buffer overrun".to_string(), t0);
        notices.push("buffer overrun".to_string(), t0 + StdDuration::from_secs(2));
        assert_eq!(notices.items.len(), 1, "el mismo aviso no se duplica");
        // Sigue vivo tras la última renovación: caduca a t0 + 8s + 2s.
        assert!(
            notices
                .active(t0 + StdDuration::from_secs(9))
                .contains("overrun"),
            "renovar alarga la vida del aviso"
        );
    }

    #[test]
    fn notices_ring_evicts_oldest_and_expires() {
        use std::time::Duration as StdDuration;
        let mut notices = Notices::default();
        let t0 = std::time::Instant::now();
        for i in 0..Notices::CAP + 2 {
            notices.push(format!("error-{i}"), t0);
        }
        // CAP=3: quedan los 3 más recientes.
        assert_eq!(notices.items.len(), Notices::CAP);
        assert!(!notices.active(t0).contains("error-0"));
        assert!(notices.active(t0).contains("error-4"));

        // Pasada la caducidad el pie queda vacío.
        assert!(
            notices
                .active(t0 + Notices::LIFETIME + StdDuration::from_secs(1))
                .is_empty(),
            "los avisos caducan solos"
        );
    }

    #[test]
    fn renders_all_views_without_panic() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.now_playing = Some(sample_track());
        app.history = vec![HistoryEntry {
            track_id: 1,
            played_at: "2026-08-02 12:00:00".to_string(),
            source: Source::YouTube,
            duration: Some(354_000),
            title: "Bohemian Rhapsody".to_string(),
            artist_name: Some("Queen".to_string()),
            play_count: 1,
        }];
        app.sources = vec![Source::YouTube];
        app.settings = Some(SettingsForm::new(Default::default()));
        app.search.results = vec![sample_track()];
        app.search.list_state.select(Some(0));
        app.related.tracks = vec![sample_track()];

        let backend = TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        for view in View::ALL {
            app.view = view;
            terminal.draw(|f| app.render(f)).unwrap();
        }

        // Los atajos mapean a todas las vistas
        let expect = [
            ('1', View::NowPlaying),
            ('2', View::Related),
            ('3', View::Search),
            ('4', View::Sources),
            ('5', View::Metadata),
            ('6', View::History),
            ('7', View::Settings),
        ];
        for (digit, view) in expect {
            let key = KeyEvent::new(KeyCode::Char(digit), KeyModifiers::SHIFT);
            app.on_key(key);
            assert_eq!(app.view, view, "atajo Shift+{digit}");
        }
    }

    #[test]
    fn digits_do_not_navigate_but_shift_does() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);

        // Un dígito suelto nunca cambia de vista (ni en vistas de solo lectura).
        app.view = View::NowPlaying;
        let key = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
        app.on_key(key);
        assert_eq!(app.view, View::NowPlaying, "el dígito suelto no navega");

        // Shift+3 como dígito+SHIFT (protocolo kitty) navega a Search.
        let key = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::SHIFT);
        app.on_key(key);
        assert_eq!(app.view, View::Search, "Shift+3 (dígito) debe ir a Search");

        // Shift+3 como símbolo ('#') es la forma típica de los terminales sin kitty.
        app.view = View::Sources;
        let key = KeyEvent::new(KeyCode::Char('#'), KeyModifiers::NONE);
        app.on_key(key);
        assert_eq!(app.view, View::Search, "Shift+3 (#) debe ir a Search");
        let key = KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE);
        app.on_key(key);
        assert_eq!(
            app.view,
            View::NowPlaying,
            "Shift+1 (!) debe ir a Now Playing"
        );

        // En Search los dígitos quedan como texto de la consulta.
        app.view = View::Search;
        let key = KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE);
        app.on_key(key);
        assert_eq!(app.view, View::Search, "el dígito debe quedar en el input");
        assert_eq!(app.search.text(), "4");
    }

    #[test]
    fn q_quits_only_in_read_only_views() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);

        app.view = View::NowPlaying;
        app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.should_quit, "q debe salir desde vistas de solo lectura");

        // En Search, la 'q' es una letra más de la consulta.
        let (tx2, _rx2) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx2);
        app.view = View::Search;
        app.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(
            !app.should_quit,
            "q no debe salir desde el input de búsqueda"
        );
        assert_eq!(app.search.text(), "q");
    }

    #[test]
    fn toggling_autoplay_notifies_backend() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        assert!(app.autoplay, "el autoplay arranca activado");

        app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(!app.autoplay);
        match rx.try_recv() {
            Ok(BackendCommand::SetAutoplay(false)) => {}
            other => panic!("esperaba SetAutoplay(false), llegó {other:?}"),
        }
    }

    #[test]
    fn nowplaying_arrows_navigate_recommendations() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.related.tracks = vec![sample_track(), sample_track(), sample_track()];
        app.related.list_state.select(Some(0));

        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.related.list_state.selected(), Some(1));
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.related.list_state.selected(), Some(2));
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.related.list_state.selected(), Some(1));
    }

    #[test]
    fn nowplaying_enter_plays_selected_recommendation() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.related.tracks = vec![sample_track()];
        app.related.list_state.select(Some(0));

        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut cmds = Vec::new();
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        assert!(
            cmds.iter().any(|c| matches!(c, BackendCommand::Play(_))),
            "Enter debe reproducir la selección: {cmds:?}"
        );
    }

    #[test]
    fn nowplaying_enter_without_selection_loads_related() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.now_playing = Some(sample_track());

        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match rx.try_recv() {
            Ok(BackendCommand::LoadRelated(..)) => {}
            other => panic!("esperaba LoadRelated, llegó {other:?}"),
        }
    }

    #[test]
    fn new_playback_track_auto_loads_related() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;

        let status = PlaybackStatus {
            track: Some(sample_track()),
            state: PlaybackState::Playing,
            position: std::time::Duration::ZERO,
            duration: None,
            stalled: false,
        };
        app.on_backend(BackendEvent::PlaybackStarted {
            status,
            stats: vec![],
        });
        // Se pide además la miniatura del track antes de los recomendados.
        let mut found_related = false;
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                BackendCommand::LoadRelated(..) => found_related = true,
                BackendCommand::LoadHistory => {}
                BackendCommand::Thumbnail(_) => {}
                BackendCommand::SetAutoplayQueue(queue) if queue.is_empty() => {}
                other => panic!("comando inesperado {other:?}"),
            }
        }
        assert!(found_related, "debe pedir recomendaciones del nuevo track");
    }

    #[test]
    fn shift_d_skips_to_next_recommendation() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.related.tracks = vec![rec_track("a"), rec_track("b"), rec_track("c")];
        app.now_playing = Some(rec_track("a"));

        // Kitty: 'D' mayúscula con SHIFT. La cola la resuelve el backend (la
        // lista local solo es la vista), así que el comando se delega siempre.
        app.on_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        assert!(
            sent_skip(&mut rx, BackendCommand::NextTrack),
            "Shift+D debe pedir la siguiente de la cola"
        );
    }

    #[test]
    fn shift_d_wraps_to_first_after_last() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.related.tracks = vec![rec_track("a"), rec_track("b"), rec_track("c")];
        app.now_playing = Some(rec_track("c"));

        // Legacy: 'D' mayúscula sin modificador (símbolo desplazado), también debe saltar.
        app.on_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE));
        assert!(sent_skip(&mut rx, BackendCommand::NextTrack));
    }

    #[test]
    fn shift_a_returns_to_previous() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.related.tracks = vec![rec_track("a"), rec_track("b"), rec_track("c")];
        app.now_playing = Some(rec_track("b"));

        app.on_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert!(
            sent_skip(&mut rx, BackendCommand::PrevTrack),
            "Shift+A debe pedir la anterior de la cola"
        );
    }

    #[test]
    fn shift_a_wraps_from_first_to_last() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.related.tracks = vec![rec_track("a"), rec_track("b"), rec_track("c")];
        app.now_playing = Some(rec_track("a"));

        app.on_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));
        assert!(sent_skip(&mut rx, BackendCommand::PrevTrack));
    }

    #[test]
    fn skip_delegates_to_backend_even_without_local_list() {
        // El anclaje y el recorrido de la cola viven en el backend (cola
        // estable entre canciones): la lista local vacía (buffering de la
        // canción nueva) no bloquea el salto.
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.now_playing = Some(rec_track("b"));
        app.related.list_state.select(Some(0));

        app.on_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        assert!(
            sent_skip(&mut rx, BackendCommand::NextTrack),
            "el salto se delega al backend aunque la lista local esté vacía"
        );
    }

    #[test]
    fn skip_without_recommendations_still_delegates() {
        // Sin cola local ni canción: el comando se envía igualmente y el
        // backend responde el aviso (la cola del backend puede tener la
        // anterior aunque la vista esté limpia).
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;

        app.on_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT));
        assert!(sent_skip(&mut rx, BackendCommand::NextTrack));
    }

    #[test]
    fn same_track_does_not_reload_related() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;

        // Primer playback: carga recomendaciones.
        let status = PlaybackStatus {
            track: Some(sample_track()),
            state: PlaybackState::Playing,
            position: std::time::Duration::ZERO,
            duration: None,
            stalled: false,
        };
        app.on_backend(BackendEvent::Playback(status));
        // Primer playback encola: miniatura + recomendados. Se drena la cola.
        while rx.try_recv().is_ok() {}

        // El mismo track de nuevo (p. ej. tick de progreso): no recarga.
        let status2 = PlaybackStatus {
            track: Some(sample_track()),
            state: PlaybackState::Playing,
            position: std::time::Duration::from_secs(5),
            duration: None,
            stalled: false,
        };
        app.on_backend(BackendEvent::Playback(status2));
        assert!(rx.try_recv().is_err(), "no debe recargar recomendaciones");
    }

    #[test]
    fn stale_related_response_is_ignored() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.now_playing = Some(sample_track());

        let mut other = sample_track();
        other.title = "Otra canción".to_string();
        app.on_backend(BackendEvent::Related {
            track: Box::new(other),
            related: vec![sample_track()],
            synced: None,
            generation: 1,
        });
        assert!(app.related.tracks.is_empty(), "respuesta obsoleta ignorada");
    }

    #[test]
    fn related_without_synced_shows_clean_unavailable_state() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.now_playing = Some(sample_track());
        // Flujo real: la UI pidió recomendaciones antes de recibir la respuesta.
        let _ = app.recs.request(&sample_track().identifier()).unwrap();

        app.on_backend(BackendEvent::Related {
            track: Box::new(sample_track()),
            related: vec![],
            synced: None,
            generation: 1,
        });
        assert!(app.related.synced.is_none());
        assert!(
            app.related.synced_unavailable,
            "sin LRC → estado limpio de 'letras sincronizadas no disponibles'"
        );
    }

    #[test]
    fn related_with_synced_enables_karaoke() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.now_playing = Some(sample_track());
        let _ = app.recs.request(&sample_track().identifier()).unwrap();

        app.on_backend(BackendEvent::Related {
            track: Box::new(sample_track()),
            related: vec![],
            synced: Some("[00:01.00] hola\n[00:05.00] mundo\n".to_string()),
            generation: 1,
        });
        assert!(
            app.related.synced.is_some(),
            "el LRC de LRCLIB habilita el karaoke"
        );
        assert!(!app.related.synced_unavailable);
    }

    #[test]
    fn related_with_empty_synced_is_treated_as_unavailable() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.now_playing = Some(sample_track());
        let _ = app.recs.request(&sample_track().identifier()).unwrap();

        app.on_backend(BackendEvent::Related {
            track: Box::new(sample_track()),
            related: vec![],
            synced: Some("   ".to_string()),
            generation: 1,
        });
        assert!(app.related.synced.is_none());
        assert!(
            app.related.synced_unavailable,
            "un LRC vacío no habilita el karaoke"
        );
    }

    #[test]
    fn karaoke_clock_ignores_spurious_position_resets() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);

        let track = rec_track("song-1");
        let ev = |pos: u64| PlaybackStatus {
            track: Some(track.clone()),
            state: PlaybackState::Playing,
            position: Duration::from_secs(pos),
            duration: Some(Duration::from_secs(200)),
            stalled: false,
        };

        app.on_backend(BackendEvent::PlaybackStarted {
            status: ev(10),
            stats: vec![],
        });
        assert_eq!(app.clock.position(), Duration::from_secs(10));

        // El motor reporta 0 de golpe (re-buffer del stream): no debe reiniciar.
        app.on_backend(BackendEvent::Playback(ev(0)));
        assert_eq!(
            app.clock.position(),
            Duration::from_secs(10),
            "reinicio espurio ignorado"
        );

        app.on_backend(BackendEvent::Playback(ev(15)));
        assert_eq!(app.clock.position(), Duration::from_secs(15));
    }

    #[test]
    fn karaoke_clock_resets_on_new_track_and_clears_lyrics() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.related
            .set_synced(Some(crate::domain::lyrics::SyncLyrics::parse(
                "[00:01.00] hola\n",
            )));

        let ev = |t: &Track, pos: u64| PlaybackStatus {
            track: Some(t.clone()),
            state: PlaybackState::Playing,
            position: Duration::from_secs(pos),
            duration: None,
            stalled: false,
        };
        app.on_backend(BackendEvent::PlaybackStarted {
            status: ev(&rec_track("song-1"), 90),
            stats: vec![],
        });
        assert_eq!(app.clock.position(), Duration::from_secs(90));

        // Cambia de canción: el reloj parte de cero y las letras viejas se
        // descartan mientras llegan las del track nuevo.
        app.on_backend(BackendEvent::PlaybackStarted {
            status: ev(&rec_track("song-2"), 3),
            stats: vec![],
        });
        assert_eq!(app.clock.position(), Duration::from_secs(3));
        assert!(
            app.related.synced.is_none(),
            "letras de la canción anterior descartadas"
        );
        assert!(
            !app.related.synced_unavailable,
            "sin letras pedidas aún no es un 'no disponible'"
        );
    }

    #[test]
    fn same_track_playback_started_rewinds_karaoke() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let track = rec_track("song-1");
        app.on_backend(BackendEvent::PlaybackStarted {
            status: PlaybackStatus {
                track: Some(track.clone()),
                state: PlaybackState::Playing,
                position: Duration::from_secs(120),
                duration: Some(Duration::from_secs(200)),
                stalled: false,
            },
            stats: vec![],
        });
        app.related
            .set_synced(Some(crate::domain::lyrics::SyncLyrics::parse(
                "[00:01.00] hola\n[00:02.00] mundo\n",
            )));
        app.related
            .scroll
            .advance(app.related.synced.as_ref().unwrap(), Some(1), false, 8);
        app.clock.begin_seek(Duration::from_secs(10));
        // La respuesta de Related ya llegó para esta canción (como en el flujo
        // real), así que un replay del MISMO track no debe pedirla de nuevo.
        let generation = app.recs.loading().expect("petición en vuelo").generation;
        app.on_backend(BackendEvent::Related {
            track: Box::new(rec_track("song-1")),
            related: vec![],
            synced: Some("[00:01.00] hola\n[00:02.00] mundo\n".to_string()),
            generation,
        });

        // El autoplay vuelve al MISMO track (inicio de cola): la canción se
        // repite desde cero y el reloj/letras del karaoke deben rebobinar con
        // ella, sin descartar la letra (que sigue siendo la misma canción).
        app.on_backend(BackendEvent::PlaybackStarted {
            status: PlaybackStatus {
                track: Some(track),
                state: PlaybackState::Playing,
                position: Duration::from_secs(1),
                duration: Some(Duration::from_secs(200)),
                stalled: false,
            },
            stats: vec![],
        });
        assert_eq!(app.clock.position(), Duration::from_secs(1));
        assert!(
            app.clock.pending_seek().is_none(),
            "seek pendiente cancelado"
        );
        assert!(
            app.related.synced.is_some(),
            "la misma canción conserva su letra"
        );
        assert!(
            app.related.scroll.is_empty(),
            "la ventana del karaoke se rebobina al repetir la canción"
        );
    }

    #[test]
    fn playback_without_track_clears_karaoke_and_lyrics() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.on_backend(BackendEvent::PlaybackStarted {
            status: PlaybackStatus {
                track: Some(rec_track("song-1")),
                state: PlaybackState::Playing,
                position: Duration::from_secs(90),
                duration: None,
                stalled: false,
            },
            stats: vec![],
        });
        app.related
            .set_synced(Some(crate::domain::lyrics::SyncLyrics::parse(
                "[00:01.00] hola\n",
            )));

        // La canción terminó (fin de stream / autoplay sin siguiente): el
        // evento sin track debe limpiar karaoke y letras, no dejarlas pintadas.
        app.on_backend(BackendEvent::Playback(PlaybackStatus {
            track: None,
            state: PlaybackState::Stopped,
            position: Duration::ZERO,
            duration: None,
            stalled: false,
        }));
        assert_eq!(app.clock.position(), Duration::ZERO);
        assert!(app.clock.track_key().is_none());
        assert!(app.clock.pending_seek().is_none());
        assert!(app.related.synced.is_none(), "letras limpiadas al terminar");
        assert_eq!(app.playback.state, PlaybackState::Stopped);
    }

    #[test]
    fn playback_error_clears_pending_play_and_resets_state() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.save_and_play(rec_track("song-2"));
        assert_eq!(app.playback.state, PlaybackState::Buffering);
        assert!(app.pending_track.is_some());

        // El stream de la canción pedida falló: la UI no debe quedar colgada
        // en "preparando"; vuelve a detenido y aceptará lo que el motor
        // realmente esté reproduciendo (si es que hay algo).
        app.on_backend(BackendEvent::PlaybackError(
            "stream interrumpido".to_string(),
        ));
        assert!(app.pending_track.is_none());
        assert_eq!(app.playback.state, PlaybackState::Stopped);
        assert_eq!(app.playback.track, None);
        assert!(
            app.status.as_deref().unwrap_or("").contains("Reproducción"),
            "el fallo se muestra en la línea de estado: {:?}",
            app.status
        );
    }

    #[test]
    fn karaoke_clock_resyncs_after_seek() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let track = rec_track("song-1");
        let ev = |pos: u64| PlaybackStatus {
            track: Some(track.clone()),
            state: PlaybackState::Playing,
            position: Duration::from_secs(pos),
            duration: None,
            stalled: false,
        };
        app.on_backend(BackendEvent::PlaybackStarted {
            status: ev(100),
            stats: vec![],
        });

        // Seek hacia atrás: pendiente de resincronizar. El motor aún reporta
        // la posición previa mientras pre-descarga el salto (el audio sigue
        // sonando ahí), así que el karaoke sigue ese reloj real en vez de
        // quedarse congelado en el objetivo.
        app.clock.begin_seek(Duration::from_secs(50));
        app.on_backend(BackendEvent::Playback(ev(100)));
        assert_eq!(
            app.clock.position(),
            Duration::from_secs(100),
            "sin salto ejecutado el karaoke sigue la posición real del audio"
        );
        assert!(
            app.clock.pending_seek().is_some(),
            "el seek sigue pendiente hasta que el motor refleje el salto"
        );

        // El motor ya está en el objetivo: el seek termina y vuelve el guard.
        app.on_backend(BackendEvent::Playback(ev(50)));
        assert_eq!(app.clock.position(), Duration::from_secs(50));
        assert!(app.clock.pending_seek().is_none(), "seek resuelto");

        // Reinicio espurio tras el seek: ignorado de nuevo.
        app.on_backend(BackendEvent::Playback(ev(0)));
        assert_eq!(app.clock.position(), Duration::from_secs(50));
    }

    #[test]
    fn backward_seek_keeps_clock_on_audio_until_jump_confirms() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.playback.position = Duration::from_secs(5);
        app.clock.update(
            Some("seed-1"),
            Duration::from_secs(5),
            std::time::Instant::now(),
        );
        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));

        // El reloj no salta al objetivo todavía: el audio sigue en 5s hasta
        // que el motor ejecute el salto y confirme la nueva posición.
        assert_eq!(app.clock.position(), Duration::from_secs(5));
        assert_eq!(
            app.clock.pending_seek().map(|s| s.target),
            Some(Duration::ZERO)
        );
        assert!(matches!(rx.try_recv(), Ok(BackendCommand::Seek(0, _))));
    }

    #[test]
    fn seek_completed_event_confirms_the_clock_without_waiting_for_a_sample() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let track = rec_track("song-1");
        app.on_backend(BackendEvent::PlaybackStarted {
            status: PlaybackStatus {
                track: Some(track.clone()),
                state: PlaybackState::Playing,
                position: Duration::from_secs(100),
                duration: None,
                stalled: false,
            },
            stats: vec![],
        });
        // El usuario pide retroceder a 20s.
        app.clock.begin_seek(Duration::from_secs(20));
        app.on_backend(BackendEvent::SeekStarted);
        assert_eq!(app.playback.state, PlaybackState::Seeking);
        // El audio real aún no se movió (el karaoke sigue en 100s).
        assert_eq!(app.clock.position(), Duration::from_secs(100));

        // Backend confirma el salto REAL: el reloj se ancla en el objetivo.
        app.on_backend(BackendEvent::SeekCompleted);
        assert_eq!(app.clock.position(), Duration::from_secs(20));
        assert!(app.clock.pending_seek().is_none(), "seek confirmado");
        assert_eq!(app.playback.state, PlaybackState::Playing);
    }

    #[test]
    fn seek_failed_event_cancels_pending_seek_and_keeps_real_position() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let track = rec_track("song-1");
        app.on_backend(BackendEvent::PlaybackStarted {
            status: PlaybackStatus {
                track: Some(track.clone()),
                state: PlaybackState::Playing,
                position: Duration::from_secs(90),
                duration: None,
                stalled: false,
            },
            stats: vec![],
        });
        app.clock.begin_seek(Duration::from_secs(10));

        // El backend no pudo buscar hacia atrás: el audio sigue en 90s.
        app.on_backend(BackendEvent::SeekFailed);
        assert!(
            app.clock.pending_seek().is_none(),
            "deseo pendiente cancelado"
        );
        assert_eq!(
            app.clock.position(),
            Duration::from_secs(90),
            "audio sin mover"
        );
    }

    #[test]
    fn karaoke_now_extrapolates_while_playing_and_freezes_paused() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let track = rec_track("song-1");

        let playing = PlaybackStatus {
            track: Some(track.clone()),
            state: PlaybackState::Playing,
            position: Duration::from_secs(42),
            duration: Some(Duration::from_secs(200)),
            stalled: false,
        };
        app.on_backend(BackendEvent::PlaybackStarted {
            status: playing,
            stats: vec![],
        });
        assert_eq!(app.clock.position(), Duration::from_secs(42));
        // Mientras se reproduce, el karaoke extrapola desde la última muestra:
        // nunca se queda por detrás de la posición reportada.
        assert!(
            app.karaoke_now() >= app.clock.position(),
            "extrapola el tiempo real entre eventos del motor"
        );

        // Pausado: el audio está congelado, la extrapolación se detiene.
        let paused = PlaybackStatus {
            track: Some(track),
            state: PlaybackState::Paused,
            position: Duration::from_secs(42),
            duration: Some(Duration::from_secs(200)),
            stalled: false,
        };
        app.on_backend(BackendEvent::Playback(paused));
        assert_eq!(app.karaoke_now(), Duration::from_secs(42));
    }

    #[test]
    fn karaoke_clock_rebases_on_every_sample_not_only_when_position_advances() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let track = rec_track("song-1");
        let ev = |state: PlaybackState, pos: u64| PlaybackStatus {
            track: Some(track.clone()),
            state,
            position: Duration::from_secs(pos),
            duration: Some(Duration::from_secs(200)),
            stalled: false,
        };

        app.on_backend(BackendEvent::PlaybackStarted {
            status: ev(PlaybackState::Playing, 50),
            stats: vec![],
        });

        // Simula una pausa larga: llega una muestra MUCHO después pero con la
        // misma posición (el ticker reenvía el estado mientras está en pausa).
        // Ancla vieja forzada (antes solo era posible tocar el campo interno).
        app.clock
            .force_anchor(std::time::Instant::now() - std::time::Duration::from_secs(600));
        app.on_backend(BackendEvent::Playback(ev(PlaybackState::Paused, 50)));

        // La extrapolación debe quedar anclada a la muestra recién recibida,
        // no saltar 10 minutos: si lo hiciera, `finished` se dispararía y el
        // karaoke aparecería vacío tras reanudar.
        assert!(
            app.karaoke_now() < Duration::from_secs(60),
            "el reloj no extrapola a través de una pausa larga (ahora={:?})",
            app.karaoke_now()
        );
    }

    #[test]
    fn save_and_play_clears_stale_lyrics_immediately() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.related
            .set_synced(Some(crate::domain::lyrics::SyncLyrics::parse(
                "[00:01.00] letra vieja\n",
            )));
        app.playback.state = PlaybackState::Playing;
        app.playback.position = Duration::from_secs(90);

        app.save_and_play(rec_track("song-2"));

        // Al cambiar de canción no debe quedar ni una muestra de la anterior:
        // la plantilla del karaoke se descarta de inmediato, no al llegar la
        // respuesta del backend (que tarda en resolver el stream).
        assert!(app.related.synced.is_none(), "LRC anterior descartado");
        assert_eq!(app.clock.position(), Duration::ZERO, "reloj reiniciado");
        assert!(
            app.clock.track_key().is_none(),
            "sin reloj para la canción vieja"
        );
        assert_eq!(app.playback.state, PlaybackState::Buffering);
    }

    #[test]
    fn pending_song_rejects_old_playback_and_lyrics() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let old = rec_track("song-old");
        let new = rec_track("song-new");
        app.now_playing = Some(old.clone());
        app.related
            .set_synced(Some(crate::domain::lyrics::SyncLyrics::parse(
                "[00:01] letra vieja\n",
            )));

        app.save_and_play(new.clone());
        assert_eq!(
            app.now_playing.as_ref().map(Track::identifier),
            Some(new.identifier())
        );
        assert!(app.related.synced.is_none());

        // Un tick del motor anterior no puede recuperar su estado durante el
        // buffering de la nueva canción.
        app.on_backend(BackendEvent::Playback(PlaybackStatus {
            track: Some(old.clone()),
            state: PlaybackState::Playing,
            position: Duration::from_secs(20),
            duration: Some(Duration::from_secs(200)),
            stalled: false,
        }));
        assert_eq!(
            app.now_playing.as_ref().map(Track::identifier),
            Some(new.identifier())
        );

        // Tampoco se acepta la respuesta LRC tardía de la canción anterior.
        app.on_backend(BackendEvent::Related {
            track: Box::new(old),
            related: vec![],
            synced: Some("[00:01] letra vieja\n".to_string()),
            generation: 1,
        });
        assert!(app.related.synced.is_none());
    }

    #[test]
    fn switching_to_related_view_reuses_loaded_recommendations() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let id = "song".to_string();
        app.now_playing = Some(rec_track(&id));
        // Carga completada: las recomendaciones de `song` ya están en la sesión.
        app.related.tracks = vec![rec_track("a"), rec_track("b")];
        app.recs.request(&id);
        app.recs.complete(&id, 1);
        while rx.try_recv().is_ok() {}

        app.switch_view(View::Related);
        assert_eq!(app.view, View::Related);
        assert_eq!(
            app.related.tracks.len(),
            2,
            "muestra las recomendaciones ya cargadas, sin regenerar"
        );
        assert!(
            rx.try_recv().is_err(),
            "cambiar de vista NO regenera recomendaciones"
        );
    }

    #[test]
    fn w_s_and_arrows_navigate_the_related_list() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::Related;
        app.related.tracks = vec![rec_track("a"), rec_track("b"), rec_track("c")];

        app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(
            app.related.list_state.selected(),
            Some(0),
            "s baja al primer ítem"
        );
        app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!(app.related.list_state.selected(), Some(1));
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.related.list_state.selected(), Some(2));
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.related.list_state.selected(),
            Some(0),
            "wrap hacia delante"
        );
        app.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        assert_eq!(
            app.related.list_state.selected(),
            Some(2),
            "wrap hacia atrás"
        );
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.related.list_state.selected(), Some(1));
    }

    #[test]
    fn related_navigation_keys_never_send_load_related() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::Related;
        app.now_playing = Some(rec_track("song"));
        app.related.tracks = vec![rec_track("a"), rec_track("b")];
        while rx.try_recv().is_ok() {}

        for code in [
            KeyCode::Char('w'),
            KeyCode::Char('s'),
            KeyCode::Up,
            KeyCode::Down,
        ] {
            app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        let mut cmds = Vec::new();
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        assert!(
            cmds.is_empty(),
            "navegar la lista no envía comandos: {cmds:?}"
        );
    }

    #[test]
    fn seek_keys_clamp_backwards_to_zero() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.view = View::NowPlaying;
        app.now_playing = Some(rec_track("song"));
        app.playback.duration = Some(Duration::from_secs(200));
        app.clock.update(
            Some("song"),
            Duration::from_secs(5),
            std::time::Instant::now(),
        );

        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        match rx.try_recv() {
            Ok(BackendCommand::Seek(0, for_track)) => assert_eq!(for_track, "song"),
            other => panic!("esperaba Seek(0, \"song\"), llegó {other:?}"),
        }
    }

    #[test]
    fn seek_keys_clamp_forward_to_duration() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        app.now_playing = Some(rec_track("song"));
        app.playback.duration = Some(Duration::from_secs(200));
        app.clock.update(
            Some("song"),
            Duration::from_secs(190),
            std::time::Instant::now(),
        );

        app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        match rx.try_recv() {
            Ok(BackendCommand::Seek(200, _)) => {}
            other => panic!("esperaba Seek(200, ...), llegó {other:?}"),
        }
    }

    #[test]
    fn seek_without_track_clamps_to_zero_without_panicking() {
        let (tx, mut rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);

        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        match rx.try_recv() {
            Ok(BackendCommand::Seek(0, _)) => {}
            other => panic!("esperaba Seek(0, ...), llegó {other:?}"),
        }
    }

    #[test]
    fn late_related_response_for_a_replayed_track_does_not_stomp_the_new_session() {
        let (tx, _rx) = unbounded_channel::<BackendCommand>();
        let mut app = App::new(tx);
        let id = "song".to_string();

        // Primera sesión de `song`: carga completada.
        app.now_playing = Some(rec_track(&id));
        let gen1 = app.recs.request(&id).unwrap();
        assert!(app.recs.complete(&id, gen1));

        // El usuario pasó por otra canción y vuelve a `song`: sesión nueva.
        app.recs.on_track_changed();
        let gen2 = app.recs.request(&id).unwrap();
        assert_ne!(gen1, gen2, "la sesión nueva tiene generación distinta");

        // La respuesta de la PRIMERA sesión llega tarde: se descarta sin
        // poblar la lista ni liberar la petición en curso.
        app.on_backend(BackendEvent::Related {
            track: Box::new(rec_track(&id)),
            related: vec![rec_track("stale")],
            synced: None,
            generation: gen1,
        });
        assert!(
            app.related.tracks.is_empty(),
            "la sesión nueva no recibe contenido de la anterior"
        );
        assert!(app.recs.is_loading(), "y la petición en curso sigue viva");

        // La respuesta de la sesión en curso sí aplica.
        app.on_backend(BackendEvent::Related {
            track: Box::new(rec_track(&id)),
            related: vec![rec_track("fresh")],
            synced: None,
            generation: gen2,
        });
        assert_eq!(app.related.tracks.len(), 1);
        assert_eq!(app.related.tracks[0].identifier(), "fresh");
        assert!(!app.recs.is_loading());
    }

    #[tokio::test]
    #[ignore = "requiere red (reproduce un stream de YouTube real)"]
    async fn search_select_records_history() {
        use std::collections::HashMap;
        use std::time::Duration;

        use crate::infrastructure::config::Config;
        use crate::infrastructure::db::Db;

        use super::super::backend::{spawn_backend, Backend};
        use super::super::event::UiEvent;

        let dir = tempfile::tempdir().unwrap();
        let db = Db::connect(dir.path().join("music.db").to_str().unwrap())
            .await
            .unwrap();

        let mut seed = Track::new(
            "Bohemian Rhapsody".to_string(),
            vec![Artist::new("Queen".to_string(), None, None, None)],
            Source::YouTube,
        );
        seed.external_id = Some("yt-seed".to_string());
        let mut ids = HashMap::new();
        ids.insert(Source::YouTube, "yt-seed".to_string());
        db.upsert_track(&seed, &ids).await.unwrap();

        let backend = Backend::new(db.clone(), Config::default());
        let (backend_tx, backend_rx) = spawn_backend(backend);
        let (ui_tx, ui_rx) = unbounded_channel();

        let mut app = App::new(backend_tx);
        let handle = tokio::spawn(async move {
            let backend = ratatui::backend::TestBackend::new(120, 40);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            app.run(&mut terminal, ui_rx, backend_rx).await
        });

        let key = |code: KeyCode, shift: bool| {
            KeyEvent::new(
                code,
                if shift {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                },
            )
        };

        ui_tx
            .send(UiEvent::Key(key(KeyCode::Char('3'), true)))
            .unwrap();
        for c in "bohemian".chars() {
            ui_tx
                .send(UiEvent::Key(key(KeyCode::Char(c), false)))
                .unwrap();
        }
        ui_tx
            .send(UiEvent::Key(key(KeyCode::Enter, false)))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        ui_tx
            .send(UiEvent::Key(key(KeyCode::Enter, false)))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        ui_tx
            .send(UiEvent::Key(key(KeyCode::Char('q'), false)))
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("la app no terminó")
            .unwrap()
            .unwrap();

        let history = db.recent_history(10).await.unwrap();
        assert_eq!(history.len(), 1, "debe registrarse la selección");
        assert_eq!(history[0].title, "Bohemian Rhapsody");
    }
}
