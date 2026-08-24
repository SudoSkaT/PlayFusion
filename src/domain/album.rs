//! Modelo de dominio: Álbum.

use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Album {
    pub id: i64,
    pub title: String,
    pub release_date: Option<NaiveDate>,
    pub cover: Option<String>,
    pub label: Option<String>,
    pub artist_ids: Vec<i64>,
}

impl Album {
    pub fn new(
        title: String,
        release_date: Option<NaiveDate>,
        cover: Option<String>,
        label: Option<String>,
    ) -> Self {
        Self {
            id: 0,
            title,
            release_date,
            cover,
            label,
            artist_ids: Vec::new(),
        }
    }
}
