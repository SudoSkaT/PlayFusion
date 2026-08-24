//! Contrato de streaming: resolución de audio independiente del proveedor.
//!
//! Un [`StreamProvider`] sabe transformar un [`Track`] en una
//! [`StreamResolution`]. No controla UI, ni cola, ni reproduce audio, ni
//! conoce el renderer: solo resuelve y reporta fallos estructurados.

use std::time::Instant;

use async_trait::async_trait;

use crate::domain::{source::Source, track::Track, stream::StreamResolution};
use crate::media::failure::ResolutionError;

/// Contexto de una resolución individual.
///
/// El resolver aplica su propio timeout alrededor de `resolve`; el deadline
/// queda expuesto por si el adaptador quiere acotar sub-operaciones.
#[derive(Debug, Clone, Default)]
pub struct ResolveContext {
    /// Instante límite opcional para toda la operación.
    pub deadline: Option<Instant>,
}

impl ResolveContext {
    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            deadline: Some(deadline),
        }
    }
}

/// Proveedor de streams de audio.
///
/// Implementado por cada adaptador (`providers/*`) y registrado en el
/// [`crate::media::StreamRegistry`]. Las decisiones de ORDEN y SALUD no viven
/// aquí: las toman router/política/breaker sobre metadatos declarados
/// ([`Self::priority`]) y resultados observados.
#[async_trait]
pub trait StreamProvider: std::fmt::Debug + Send + Sync {
    /// Identificador estable del proveedor (métricas, logs, configuración).
    fn id(&self) -> &'static str;

    /// Origen de los tracks que puede resolver.
    fn source(&self) -> Source;

    /// Prioridad estática: MAYOR se intenta antes (default 100).
    ///
    /// La ordenación dinámica (salud, historial reciente) la aplica el router
    /// encima de este valor base.
    fn priority(&self) -> u32 {
        100
    }

    /// `true` si este proveedor puede INTENTAR resolver el track.
    ///
    /// Debe ser puro y barato (sin I/O): solo inspección del track frente a
    /// las capacidades del proveedor.
    fn supports(&self, track: &Track) -> bool {
        track.source == self.source()
            && track
                .external_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty())
    }

    /// Resuelve el stream de audio del track.
    ///
    /// Devuelve SIEMPRE `Err(ResolutionError)` clasificado: nunca un string
    /// opaco ni un error sin categoría. `Ok` implica una resolución VALIDADA
    /// estructuralmente (URI presente) y con vigencia honesta (`expires_at`
    /// cuando el proveedor la conoce; nunca inventada).
    async fn resolve(
        &self,
        track: &Track,
        ctx: &ResolveContext,
    ) -> Result<StreamResolution, ResolutionError>;
}
