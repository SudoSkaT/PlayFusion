//! Capa de Analysis: PCM → features musicales → parámetros listos para
//! visualización (spec §19-§21).
//!
//! Flujo:
//!
//! ```text
//! RodioBackend (hilo de audio)
//!   └─ TapSource: copia las muestras decodificadas a un anillo SPSC acotado
//!        ↓ (nunca bloquea el audio; backpressure = descartar lo nuevo si está lleno)
//! AudioAnalysisEngine (HILO DEDICADO, spec §34/35)
//!   └─ ventana/Hann → FFT → bandas + centroid + flujo
//!        → onset adaptativo → beat/BPM (autocorrelación) → suavizado EMA
//!        ↓
//! FeatureBus: snapshot inmutable [`AudioFeatures`] (Arc swap; lectores sin
//! contención apreciable a 30 Hz frente a ~90 Hz de publicación).
//!
//! El análisis NUNCA toca la ruta del audio más allá de una memcpy por muestra.
//! El reloj musical sigue siendo el PositionClock del playback: los timestamps
//! de features marcan tiempo de STREAM procesado, y el consumidor visual los
//! combina con la posición real de reproducción (spec §17).
//! ```

pub mod beat;
pub mod bands;
pub mod engine;
pub mod fft;
pub mod features;
pub mod onset;
pub mod ring;
pub mod rms;
pub mod smoother;
pub mod tap;
#[cfg(test)]
pub mod test_support;

pub use engine::{AnalysisConfig, AnalysisRuntime, PcmTap, StreamMeta};
pub use tap::TapSource;
pub use features::{AudioFeatures, FeatureBus};
pub use ring::SpScRing;
pub use smoother::FeatureSmoother;
