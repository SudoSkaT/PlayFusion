//! Adaptadores de proveedores externos.
//!
//! Cada submódulo encapsula UN servicio externo: sus clientes, parsers,
//! mappers y modelos propios. Nada de aquí se importa desde UI, playback,
//! analysis, visualization o media — solo desde el punto de composición
//! ([`crate::api`]).

pub mod youtube;
