//! Motor de búsqueda: consulta primero la cache local (SQLite) y, si no hay
//! resultados, delega en el agregador de proveedores remotos. Además enriquece
//! el resultado con recomendaciones relacionadas (semilla = mejor coincidencia)
//! para que la búsqueda no quede limitada a versiones de una misma canción.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use crate::domain::{source::Source, track::Track};
use crate::infrastructure::db::Db;

use super::aggregator::{MetadataAggregator, SearchOutcome};

/// Máximo de recomendaciones añadidas a un resultado de búsqueda.
const SEARCH_REC_LIMIT: usize = 8;

#[derive(Debug, Clone)]
pub struct SearchEngine {
    db: Db,
    aggregator: Arc<MetadataAggregator>,
}

impl SearchEngine {
    pub fn new(db: Db, aggregator: Arc<MetadataAggregator>) -> Self {
        Self { db, aggregator }
    }

    /// Búsqueda local primero; si está vacía, busca en las fuentes remotas.
    /// Al final combina el resultado con recomendaciones relacionadas con el
    /// primer acierto (marcadas a partir de `related_from`).
    pub async fn search_tracks(&self, query: &str, limit: u32) -> Result<SearchOutcome<Track>> {
        let local = self.db.search_local(query, limit as i64).await?;
        let mut outcome = if !local.is_empty() {
            SearchOutcome {
                items: local,
                errors: Vec::new(),
                related_from: 0,
            }
        } else {
            self.aggregator.search_tracks(query, limit).await
        };

        self.enrich_with_related(query, &mut outcome).await;
        self.aggregator
            .hydrate_missing_durations(&mut outcome.items)
            .await;
        Ok(outcome)
    }

    /// Añade recomendaciones relacionadas con la búsqueda (semilla: el primer
    /// resultado con id externo; si los resultados locales no lo tienen —p. ej.
    /// guardados por versiones antiguas—, se busca remotamente la consulta para
    /// obtener una semilla). Descarta duplicados.
    async fn enrich_with_related(&self, query: &str, outcome: &mut SearchOutcome<Track>) {
        let seed = outcome.items.iter().find_map(|t| t.external_id.clone());
        let seed = if let Some(seed) = seed {
            seed
        } else {
            let Some(seed) = self.aggregator.search_tracks(query, 1).await.items.pop() else {
                return;
            };
            let Some(id) = seed.external_id else { return };
            id
        };
        let related = self.aggregator.related(&seed).await;
        if related.is_empty() {
            return;
        }

        let mut seen: std::collections::HashSet<String> =
            outcome.items.iter().map(|t| t.identifier()).collect();
        let direct_len = outcome.items.len();
        let mut added = 0usize;
        for track in related {
            if added >= SEARCH_REC_LIMIT {
                break;
            }
            if seen.insert(track.identifier()) {
                outcome.items.push(track);
                added += 1;
            }
        }
        if added > 0 {
            outcome.related_from = direct_len;
        }
    }

    /// Persiste una canción encontrada y devuelve su `id` canónico interno.
    pub async fn save_track(
        &self,
        track: &Track,
        provider_ids: &HashMap<Source, String>,
    ) -> Result<i64> {
        self.db.upsert_track(track, provider_ids).await
    }

    pub fn aggregator(&self) -> &MetadataAggregator {
        &self.aggregator
    }
}
