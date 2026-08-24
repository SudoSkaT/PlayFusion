//! Modelo de dominio: Género / Tag.

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Genre {
    pub id: i64,
    pub name: String,
}

impl Genre {
    pub fn new(name: String) -> Self {
        Self { id: 0, name }
    }
}
