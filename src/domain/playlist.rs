//! Modelo de dominio: Playlist.

use crate::domain::track::Track;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub tracks: Vec<Track>,
}
