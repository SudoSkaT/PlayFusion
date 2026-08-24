//! Backends de reproducción de audio (Infraestructura).
//!
//! - [`output`]: salida de audio local compartida (rodio/cpal).
//! - [`rodio_backend`]: streaming HTTP decodificado con symphonia y emitido por
//!   el dispositivo de audio local.
//!
//! Ninguno de estos tipos se exporta fuera de esta capa; el enrutador de la
//! capa de Aplicación solo conoce el trait.

mod output;
mod rodio_backend;

use std::sync::Arc;

use crate::app::audio::{EventBus, PlaybackEngine};
use crate::app::playback::RouterConfig;
use crate::domain::source::Source;
use crate::infrastructure::config::Config;

pub use output::SharedOutput;
pub use rodio_backend::RodioBackend;

/// Construye los motores a partir de la configuración. `http` es el cliente
/// HTTP compartido para resolver/descargar streams. Todos los motores notifican
/// al `bus` agrupado.
pub fn build_engines(
    config: &Config,
    bus: EventBus,
    http: reqwest::Client,
) -> (RouterConfig, Option<crate::analysis::FeatureBus>) {
    let mut engines: Vec<Arc<dyn PlaybackEngine>> = Vec::new();

    // Análisis de audio opcional: un runtime por construcción de motores; su
    // Drop detiene el hilo al reconstruirse (ajustes). El FeatureBus viaja
    // hacia fuera para los consumidores (visualización Fase 7).
    let analysis = config
        .flags
        .audio_analysis
        .then(|| crate::analysis::AnalysisRuntime::spawn(crate::analysis::AnalysisConfig::default()));
    let features = analysis.as_ref().map(|rt| rt.bus());

    // Salida local para rodio. Si no hay dispositivo de audio, el backend
    // notificará el error en play(). Sus errores en caliente (underrun,
    // desconexión) van al bus agrupado, no a stderr.
    let output =
        Arc::new(SharedOutput::try_new(bus.clone()).expect("dispositivo de audio disponible"));

    engines.push(Arc::new(RodioBackend::new(
        http.clone(),
        output.clone(),
        bus.clone(),
        !config.flags.proxy,
        analysis,
    )));

    let policy = config.playback_policy.clone();
    (RouterConfig { engines, policy }, features)
}

/// Fuentes que reproducen stream HTTP directo. Todos los orígenes actuales lo son.
pub(crate) fn is_http_source(source: Source) -> bool {
    matches!(source, Source::YouTube)
}
