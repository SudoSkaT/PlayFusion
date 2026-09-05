//! Capa Media: resolución de streams independiente de proveedores.
//!
//! Piezas y responsabilidades:
//!
//! | Módulo         | Contrato                                          |
//! |----------------|---------------------------------------------------|
//! | [`failure`]    | taxonomía estructural de fallos                   |
//! | [`provider`]   | `StreamProvider`: resolver un track               |
//! | [`policy`]     | reintento/fallback acotado (PURA)                 |
//! | [`circuit`]    | salud por proveedor (breaker local)               |
//! | [`cache`]      | caché de resoluciones con expiración              |
//! | [`registry`]   | registro/enable/disable/salud                     |
//! | [`router`]     | ordenación PURA de candidatos                     |
//! | [`resolver`]   | orquestación completa (spec §8)                   |
//! | [`expiration`] | barrido y expiración próxima                      |
//!
//! Nada en este módulo conoce proveedores concretos: los adaptadores de
//! `providers/*` implementan [`provider::StreamProvider`] y el punto de
//! composición los registra.

pub mod cache;
pub mod circuit;
pub mod expiration;
pub mod failure;
pub mod policy;
pub mod provider;
pub mod registry;
pub mod resolver;
#[cfg(test)]
pub mod test_support;
pub mod transport;

// El router es puro y se usa vía `crate::media::router::order`; se exporta
// como módulo para mantener la función libre.
pub mod router;

pub use cache::{MemoryResolutionCache, ResolutionCache, TwoTierCache};
pub use circuit::{CircuitBreaker, CircuitConfig, CircuitState};
pub use expiration::ExpirationManager;
pub use failure::{FailureCategory, ResolutionError};
pub use policy::{FailurePolicy, PolicyAction};
pub use provider::{ResolveContext, StreamProvider};
pub use registry::{ProviderSnapshot, StreamRegistry};
pub use resolver::{AttemptRecord, ResolveError, ResolverConfig, StreamResolver, StreamValidator};
pub use transport::{HttpRangeStream, RangePolicy, TransportFailure};
