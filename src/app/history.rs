//! Gestión del historial de reproducción sobre la base local.

use anyhow::Result;

use crate::domain::source::Source;
use crate::infrastructure::db::Db;
use crate::infrastructure::storage::{HistoryEntry, TrackListeningStats};

#[derive(Debug, Clone)]
pub struct History {
    db: Db,
}

impl History {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Registra una reproducción de `track_id` desde `source`.
    pub async fn record(
        &self,
        track_id: i64,
        source: Source,
        duration: Option<std::time::Duration>,
    ) -> Result<()> {
        self.db
            .record_history(track_id, source, duration.map(|d| d.as_millis() as i64))
            .await
    }

    /// Devuelve las últimas `limit` reproducciones.
    pub async fn recent(&self, limit: i64) -> Result<Vec<HistoryEntry>> {
        self.db.recent_history(limit).await
    }

    pub async fn stats(&self) -> Result<Vec<TrackListeningStats>> {
        self.db.listening_stats().await
    }
}
