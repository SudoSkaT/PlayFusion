//! Taxonomía estructurada de fallos de resolución/reproducción.
//!
//! Toda clasificación de errores del sistema multimedia pasa por
//! [`FailureCategory`]; nunca strings arbitrarios. Las categorías llevan sus
//! propiedades intrínsecas (¿reintentar? ¿probar otro provider? ¿invalidar la
//! caché?) para que la política de Fase 2 solo componga decisiones, no
//! re-clasifique.

use crate::domain::source::Source;

/// Categoría estructural de un fallo.
///
/// El significado de cada variante es contractual: los proveedores mapean sus
/// errores nativos a estas categorías y las capas superiores (resolver,
/// recovery, métricas) deciden según los predicados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureCategory {
    /// El proveedor no puede manejar este track (sin audio, formato ajeno).
    /// Determinista: repetir contra el mismo proveedor no cambia nada.
    Unsupported,
    /// Error de red (DNS, conexión, lectura del stream).
    NetworkFailure,
    /// La operación excedió su tiempo límite.
    Timeout,
    /// Hace falta autenticación/token (p. ej. PO token caducado). Re-resolver
    /// puede renovarla.
    AuthenticationRequired,
    /// El proveedor está caído o fuera de servicio (circuit abierto, 5xx
    /// sostenido). Reintentar ya mismo es inútil; cooldown lo gestiona la
    /// política.
    ProviderUnavailable,
    /// La respuesta del proveedor violó el formato esperado (cambio de
    /// protocolo, HTML donde se esperaba JSON...). Determinista a corto plazo.
    InvalidResponse,
    /// El stream resuelto caducó (URL muerta): hay que re-resolver.
    StreamExpired,
    /// Fallo durante la reproducción en curso (decodificación, corte a mitad).
    PlaybackFailure,
    /// Límite de peticiones alcanzado (429 / cuota deslizante): backoff antes
    /// de reintentar.
    RateLimited,
    /// El servidor restringe el stream más allá de un límite posicional por
    /// URL (techo de entrega según el contexto de resolución; NO es cuota por
    /// IP). Re-resolver con otro contexto puede evitarlo.
    StreamRestricted,
    /// Sin clasificación posible. Tratamiento conservador: fallback sí,
    /// retry no.
    Unknown,
}

impl FailureCategory {
    /// `true` si repetir el intento (tras backoff si procede) tiene sentido:
    /// fallos transitorios o auto-renovables (auth, expiración). El fallback a
    /// otro proveedor NO se decide aquí: es política del resolver según los
    /// proveedores disponibles tras cada fallo.
    pub fn is_retryable(self) -> bool {
        match self {
            FailureCategory::NetworkFailure
            | FailureCategory::Timeout
            | FailureCategory::AuthenticationRequired
            | FailureCategory::StreamExpired
            | FailureCategory::PlaybackFailure
            | FailureCategory::RateLimited
            | FailureCategory::StreamRestricted => true,
            // Deterministas o sin remedio inmediato.
            FailureCategory::Unsupported
            | FailureCategory::ProviderUnavailable
            | FailureCategory::InvalidResponse
            | FailureCategory::Unknown => false,
        }
    }

    /// `true` si este fallo invalida una resolución guardada (la URI cacheada
    /// quedó inutilizable o sospechosa).
    pub fn invalidates_cache(self) -> bool {
        matches!(
            self,
            FailureCategory::StreamExpired
                | FailureCategory::AuthenticationRequired
                | FailureCategory::PlaybackFailure
                | FailureCategory::StreamRestricted
        )
    }
}

impl std::fmt::Display for FailureCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            FailureCategory::Unsupported => "no soportado",
            FailureCategory::NetworkFailure => "fallo de red",
            FailureCategory::Timeout => "timeout",
            FailureCategory::AuthenticationRequired => "autenticación requerida",
            FailureCategory::ProviderUnavailable => "proveedor no disponible",
            FailureCategory::InvalidResponse => "respuesta inválida",
            FailureCategory::StreamExpired => "stream caducado",
            FailureCategory::PlaybackFailure => "fallo de reproducción",
            FailureCategory::RateLimited => "límite de peticiones",
            FailureCategory::StreamRestricted => "stream restringido por el servidor",
            FailureCategory::Unknown => "desconocido",
        };
        f.write_str(label)
    }
}

/// Error tipado devuelto por un proveedor de stream al resolver un track.
///
/// Conserva SIEMPRE la categoría estructural y el origen: el resolver nunca
/// oculta el motivo original ni lo degrada a un string opaco.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{provider}: {category}: {message}")]
pub struct ResolutionError {
    pub category: FailureCategory,
    pub provider: Source,
    pub message: String,
}

impl ResolutionError {
    pub fn new(
        category: FailureCategory,
        provider: Source,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            provider,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use FailureCategory as C;

    #[test]
    fn transient_categories_are_retryable() {
        assert!(C::NetworkFailure.is_retryable());
        assert!(C::Timeout.is_retryable());
        assert!(C::RateLimited.is_retryable());
        // Re-resolver renueva auth y URL muerta.
        assert!(C::AuthenticationRequired.is_retryable());
        assert!(C::StreamExpired.is_retryable());
        assert!(C::PlaybackFailure.is_retryable());
        // La restricción posicional se combate re-resolviendo (otro contexto).
        assert!(C::StreamRestricted.is_retryable());
    }

    #[test]
    fn restricted_stream_invalidates_the_cached_resolution() {
        assert!(C::StreamRestricted.invalidates_cache());
    }

    #[test]
    fn deterministic_categories_are_not_retryable() {
        assert!(!C::Unsupported.is_retryable());
        assert!(!C::ProviderUnavailable.is_retryable());
        assert!(!C::InvalidResponse.is_retryable());
        assert!(!C::Unknown.is_retryable());
    }

    #[test]
    fn dead_or_suspect_resolutions_invalidate_cache() {
        assert!(C::StreamExpired.invalidates_cache());
        assert!(C::PlaybackFailure.invalidates_cache());
        assert!(C::AuthenticationRequired.invalidates_cache());

        // Un fallo de red NO culpa a la resolución cacheada: puede seguir viva.
        assert!(!C::NetworkFailure.invalidates_cache());
        assert!(!C::Timeout.invalidates_cache());
        assert!(!C::RateLimited.invalidates_cache());
        assert!(!C::InvalidResponse.invalidates_cache());
        assert!(!C::Unsupported.invalidates_cache());
    }

    #[test]
    fn error_display_carries_provider_category_and_message() {
        let e = ResolutionError::new(
            C::StreamExpired,
            Source::YouTube,
            "la URL respondió 403",
        );
        assert_eq!(
            e.to_string(),
            "YouTube: stream caducado: la URL respondió 403"
        );
        assert_eq!(e.category, C::StreamExpired);
    }
}
