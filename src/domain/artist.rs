//! Modelo de dominio: Artista.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub country: Option<String>,
    pub biography: Option<String>,
    pub image: Option<String>,
    pub genres: Vec<String>,
    /// ID externo en la plataforma de origen (ej. MBID, channel id, user id).
    pub external_id: Option<String>,
    /// Duración agregada de todas sus obras conocidas (opcional, para la UI).
    pub total_duration: Option<Duration>,
}

impl Artist {
    /// Constructor sin id de base de datos (para resultados provenientes de API).
    pub fn new(
        name: String,
        country: Option<String>,
        biography: Option<String>,
        image: Option<String>,
    ) -> Self {
        Self {
            id: 0,
            name,
            country,
            biography,
            image,
            genres: Vec::new(),
            external_id: None,
            total_duration: None,
        }
    }
}
