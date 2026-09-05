//! Tipos de eventos entre la UI (entrada) y el backend de aplicación.

use crossterm::event::{KeyEvent, MouseEvent};

use std::sync::Arc;

use crate::analysis::AudioFeatures;
use crate::app::aggregator::SearchOutcome;
use crate::app::audio::PlaybackStatus;
use crate::app::thumbnail::ThumbnailState;
use crate::domain::{source::Source, track::Track};
use crate::infrastructure::config::ConfigForm;
use crate::infrastructure::storage::{HistoryEntry, PlaylistRow, TrackListeningStats};

/// Eventos de entrada del terminal hacia el loop de la UI.
#[derive(Debug)]
pub enum UiEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
}

/// Eventos producidos por el backend (capa de aplicación) hacia la UI.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // HTTP al cuadrado de variantes de tamaño desigual
pub enum BackendEvent {
    SearchResults {
        query: String,
        outcome: Box<SearchOutcome<Track>>,
    },
    TrackSaved {
        track: Box<Track>,
        internal_id: i64,
    },
    History(Vec<HistoryEntry>),
    ListeningStats(Vec<TrackListeningStats>),
    Sources(Vec<Source>),
    Settings(ConfigForm),
    Playback(PlaybackStatus),
    /// Inicio confirmado: la escucha ya fue persistida y sus estadísticas
    /// acompañan al snapshot para que todas las vistas se actualicen juntas.
    PlaybackStarted {
        status: PlaybackStatus,
        stats: Vec<TrackListeningStats>,
    },
    PlaybackError(String),
    /// Comenzó a ejecutarse un seek en el backend (estado transitorio).
    SeekStarted,
    /// Un seek se confirmó como salto REAL del audio.
    SeekCompleted,
    /// Un seek falló: el audio no cambió de posición.
    SeekFailed,
    /// Error **en caliente** del stream de audio (buffer overrun/underrun,
    /// corte de red, error de decodificación en mitad de la canción). No es un
    /// fallo terminal de reproducción: la UI lo muestra como pie de página
    /// discreto (abajo a la derecha) y sigue reproduciendo.
    StreamError(String),
    Related {
        track: Box<Track>,
        related: Vec<Track>,
        /// Letra sincronizada en formato LRC (LRCLIB): única fuente del karaoke.
        synced: Option<String>,
        /// Generación de la sesión (devuelta por `BackendCommand::LoadRelated`):
        /// la UI solo aplica la respuesta si sigue siendo la carga en vuelo.
        generation: u64,
    },
    /// Miniatura resuelta de un track (`key` = identificador estable).
    /// El estado es `Loaded`/`Failed`/`None`; la UI muestra `Loading` desde
    /// que pide la miniatura hasta que llega este evento.
    Thumbnail {
        key: String,
        state: ThumbnailState,
    },
    /// Último snapshot de análisis de audio (~15 Hz mientras suena).
    Features(Arc<AudioFeatures>),
    Playlists(Vec<PlaylistRow>),
    PlaylistTracks {
        playlist_id: i64,
        tracks: Vec<Track>,
    },
    Message(String),
    Error(String),
}
