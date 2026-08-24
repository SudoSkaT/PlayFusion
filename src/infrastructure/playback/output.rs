//! Salida de audio compartida por el motor local (rodio).
//!
//! Abre el dispositivo por defecto una sola vez y expone un [`Player`] de rodio
//! que el motor usa para encolar muestras.
//!
//! Los errores del dispositivo (underrun/overrun, desconexión) se enrutan al
//! bus de eventos de reproducción en lugar de a `stderr`: durante la TUI no se
//! escribe nada directamente sobre la interfaz y la UI decide cómo mostrarlos.

use std::sync::Arc;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::stream::supported_output_configs;
use rodio::{cpal, DeviceSinkBuilder, DeviceSinkError, MixerDeviceSink, Player};

use crate::app::audio::{EventBus, PlaybackEvent};

/// Dispositivo de audio local compartido.
pub struct SharedOutput {
    /// Mantiene viva la salida física mientras exista el dispositivo.
    _sink: MixerDeviceSink,
    player: Arc<Player>,
}

/// Traduce un error de la salida de audio a un mensaje clasificado, pensado
/// para el pie de página discreto de la UI (no para `stderr`).
fn classify_audio_error(err: &cpal::StreamError) -> String {
    match err {
        cpal::StreamError::DeviceNotAvailable => "dispositivo de audio desconectado".to_string(),
        cpal::StreamError::StreamInvalidated => "dispositivo de audio inválido".to_string(),
        cpal::StreamError::BufferUnderrun => {
            "buffer de audio del dispositivo agotado (underrun)".to_string()
        }
        cpal::StreamError::BackendSpecific { err } => {
            format!("dispositivo de audio: {err}")
        }
    }
}

/// Callback de errores del stream de audio: los convierte en un evento de
/// reproducción, con rate-limit (un underrun repetido no debe inundar la UI).
fn error_cb(bus: EventBus) -> impl FnMut(cpal::StreamError) + Send + 'static {
    use std::time::{Duration, Instant};
    let last = std::sync::Mutex::new(Instant::now() - Duration::from_secs(60));
    move |err: cpal::StreamError| {
        let now = Instant::now();
        let mut last = last.lock().unwrap();
        if now.duration_since(*last) < Duration::from_secs(3) {
            return;
        }
        *last = now;
        bus.emit(PlaybackEvent::Error(classify_audio_error(&err)));
    }
}

/// Intenta abrir `device` con el callback de error del bus (config por defecto
/// y, si falla, los formatos soportados del dispositivo).
fn open_on(device: &cpal::Device, bus: EventBus) -> Result<MixerDeviceSink, DeviceSinkError> {
    let builder = DeviceSinkBuilder::from_device(device.clone())?;
    builder
        .with_error_callback(error_cb(bus.clone()))
        .open_stream()
        .or_else(|_| {
            let mut last = None;
            for supported in supported_output_configs(device)? {
                match DeviceSinkBuilder::default()
                    .with_device(device.clone())
                    .with_supported_config(&supported)
                    .with_error_callback(error_cb(bus.clone()))
                    .open_stream()
                {
                    Ok(handle) => return Ok(handle),
                    Err(e) => last = Some(e),
                }
            }
            Err(last.unwrap_or(DeviceSinkError::NoDevice))
        })
}

impl SharedOutput {
    /// Abre el dispositivo de audio por defecto, enrutando sus errores al `bus`.
    pub fn try_new(bus: EventBus) -> Result<Self, String> {
        let builder = DeviceSinkBuilder::from_default_device()
            .map_err(|e| format!("abrir dispositivo de audio: {e}"))?;
        let sink = builder
            .with_error_callback(error_cb(bus.clone()))
            .open_stream()
            .or_else(|original_err| {
                // Igual que `open_default_sink`: si el dispositivo por defecto
                // falla, se prueba cualquier otra salida que no sea "null".
                let devices = match cpal::default_host().output_devices() {
                    Ok(devices) => devices,
                    Err(_) => return Err(original_err),
                };
                devices
                    .filter(|dev| {
                        dev.description()
                            .map(|desc| desc.driver().is_some_and(|driver| driver != "null"))
                            .unwrap_or(false)
                    })
                    .find_map(|d| open_on(&d, bus.clone()).ok())
                    .ok_or(original_err)
            })
            .map_err(|e| format!("abrir dispositivo de audio: {e}"))?;

        let mut sink = sink;
        // No escribir el mensaje de "dropping sink" a stderr al salir.
        sink.log_on_drop(false);
        let player = Arc::new(Player::connect_new(sink.mixer()));
        Ok(Self {
            _sink: sink,
            player,
        })
    }

    /// Acceso al player rodio compartido.
    pub fn player(&self) -> Arc<Player> {
        self.player.clone()
    }
}
