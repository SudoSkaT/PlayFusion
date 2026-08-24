//! Soporte de tests para la capa media (solo disponible en `cfg(test)`).
//!
//! Fakes deterministas compartidos por las suites de registro, router y
//! resolver: nada de red, resultados guionizados.

#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use crate::domain::{source::Source, stream::StreamResolution};
use crate::domain::track::Track;
use crate::media::failure::{FailureCategory, ResolutionError};
use crate::media::provider::{ResolveContext, StreamProvider};

/// Proveedor nulo: nunca se le llama (relleno tipado para snapshots).
#[derive(Debug)]
pub struct NullProvider(pub &'static str, pub Source);

#[async_trait]
impl StreamProvider for NullProvider {
    fn id(&self) -> &'static str {
        self.0
    }
    fn source(&self) -> Source {
        self.1
    }
    async fn resolve(
        &self,
        _t: &Track,
        _c: &ResolveContext,
    ) -> Result<StreamResolution, ResolutionError> {
        panic!("NullProvider no debe resolverse")
    }
}

/// Resultado guionizado de una llamada a [`FakeStreamProvider::resolve`].
#[derive(Debug, Clone)]
pub enum Step {
    /// Resolución válida con esta URI.
    Ok(&'static str),
    /// Fallo clasificado.
    Err(FailureCategory, &'static str),
    /// Duerme antes de responder `Ok` (para probar timeouts del resolver).
    SlowOk(Duration),
}

/// Proveedor falso con guion: consume los pasos en orden; agotado el guion,
/// repite el último paso. Cuenta las llamadas para aserciones.
#[derive(Debug)]
pub struct FakeStreamProvider {
    id: &'static str,
    source: Source,
    priority: u32,
    steps: Mutex<VecDeque<Step>>,
    pub calls: AtomicUsize,
}

impl FakeStreamProvider {
    pub fn new(id: &'static str, source: Source, priority: u32, steps: Vec<Step>) -> Self {
        Self {
            id,
            source,
            priority,
            steps: Mutex::new(steps.into()),
            calls: AtomicUsize::new(0),
        }
    }

    /// Siempre resuelve bien con la URI dada.
    pub fn ok(id: &'static str, source: Source, uri: &'static str) -> Self {
        Self::new(id, source, 100, vec![Step::Ok(uri)])
    }

    /// Siempre falla con la categoría dada.
    pub fn failing(
        id: &'static str,
        source: Source,
        category: FailureCategory,
        msg: &'static str,
    ) -> Self {
        Self::new(id, source, 100, vec![Step::Err(category, msg)])
    }

    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn resolution(&self, uri: &str) -> StreamResolution {
        StreamResolution::new(self.source, format!("https://cdn.fake/{uri}"))
    }
}

#[async_trait]
impl StreamProvider for FakeStreamProvider {
    fn id(&self) -> &'static str {
        self.id
    }
    fn source(&self) -> Source {
        self.source
    }
    fn priority(&self) -> u32 {
        self.priority
    }

    async fn resolve(
        &self,
        _track: &Track,
        _ctx: &ResolveContext,
    ) -> Result<StreamResolution, ResolutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Consume los pasos en orden; AGOTADO el guion, repite el último
        // (los fases "siempre X" se definen con un único paso).
        let step = {
            let mut g = self.steps.lock().unwrap();
            if g.len() > 1 {
                g.pop_front().unwrap()
            } else {
                g.front().cloned().ok_or_else(|| {
                    ResolutionError::new(
                        FailureCategory::Unknown,
                        self.source,
                        "sin guion",
                    )
                })?
            }
        };
        match step {
            Step::Ok(uri) => Ok(self.resolution(uri)),
            Step::SlowOk(d) => {
                tokio::time::sleep(d).await;
                Ok(self.resolution("slow"))
            }
            Step::Err(cat, msg) => Err(ResolutionError::new(cat, self.source, msg)),
        }
    }
}
