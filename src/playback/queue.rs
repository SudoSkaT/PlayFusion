//! Cola de reproducción formal: next/prev con vuelta, shuffle y repeat
//! (spec §15).
//!
//! Sustituye a la cola ad-hoc del autoplay del backend conservando sus
//! semánticas exactas:
//!
//! - navegación relativa al ÚLTIMO reproducido (no hay cursor propio);
//! - ancla desconocida: `next` parte del primero, `previous` del último;
//! - cola vacía → `None`;
//! - `RepeatMode::All` por defecto (la cola siempre envolvía).
//!
//! El shuffle es una permutación de índices anclada al track actual (el
//! actual queda primero al activarla); el generador es un xorshift seedeable
//! para que los tests sean deterministas.

use crate::domain::track::Track;

/// Comportamiento al llegar al extremo de la cola.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    /// Envuelve al otro extremo (comportamiento histórico de la cola).
    #[default]
    All,
    /// Se detiene en el extremo (`None`).
    Off,
}

/// Gestor de la cola de reproducción.
#[derive(Debug)]
pub struct QueueManager {
    tracks: Vec<Track>,
    last_played: Option<String>,
    shuffle: bool,
    /// Permutación de índices vigente cuando `shuffle` está activo.
    order: Vec<usize>,
    /// Estado del generador xorshift (nunca cero).
    rng: u64,
    repeat: RepeatMode,
}

impl Default for QueueManager {
    fn default() -> Self {
        Self::with_seed(0x9E3779B97F4A7C15)
    }
}

impl QueueManager {
    pub fn with_seed(seed: u64) -> Self {
        Self {
            tracks: Vec::new(),
            last_played: None,
            shuffle: false,
            order: Vec::new(),
            rng: seed | 1,
            repeat: RepeatMode::default(),
        }
    }

    // ------------------------------------------------------------ estado

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn last_played(&self) -> Option<&str> {
        self.last_played.as_deref()
    }

    /// Registra que se reprodujo un track (actualiza el ancla de navegación).
    pub fn mark_played(&mut self, id: &str) {
        self.last_played = Some(id.to_string());
    }

    // -------------------------------------------------------- mutaciones

    /// Reemplaza la cola. Conserva el ancla aunque el track ya no esté
    /// (navegará desde un extremo, como hoy). Regenera el shuffle si aplica.
    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
        if self.shuffle {
            let anchor = self.anchor_index(None);
            self.reshuffle(anchor);
        }
    }

    /// Activa/desactiva el shuffle. Al activarlo, el track actual queda
    /// primero en la permutación (no se corta la reproducción en curso).
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        match on {
            true => {
                let anchor = self.anchor_index(None);
                self.reshuffle(anchor);
            }
            false => self.order.clear(),
        }
    }

    pub fn shuffle_active(&self) -> bool {
        self.shuffle
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    // ------------------------------------------------------- navegación

    /// Track siguiente/anterior respecto al ancla (o al último reproducido).
    ///
    /// NO muta el ancla: quien reproduce decide marcar ([`Self::mark_played`]).
    /// Sí puede regenerar la permutación si la cola cambió con shuffle activo.
    pub fn pick(&mut self, forward: bool, anchor: Option<&str>) -> Option<Track> {
        if self.tracks.is_empty() {
            return None;
        }
        if self.shuffle {
            return self.pick_shuffled(forward, anchor);
        }
        let n = self.tracks.len();
        let start = self.anchor_index(anchor);
        let idx = match (start, forward) {
            (Some(i), true) => match self.repeat {
                RepeatMode::All => (i + 1) % n,
                RepeatMode::Off => {
                    if i + 1 < n {
                        i + 1
                    } else {
                        return None;
                    }
                }
            },
            (Some(i), false) => match self.repeat {
                RepeatMode::All => (i + n - 1) % n,
                RepeatMode::Off => i.checked_sub(1)?,
            },
            // Ancla desconocida: primero (hacia adelante) o último (atrás),
            // como la cola histórica.
            (None, true) => 0,
            (None, false) => n - 1,
        };
        Some(self.tracks[idx].clone())
    }

    fn pick_shuffled(&mut self, forward: bool, anchor: Option<&str>) -> Option<Track> {
        let current = self.anchor_index(anchor);
        // Permutación estructuralmente válida para ESTA cola (la regeneración
        // anclada solo ocurre en mutaciones: set_tracks/set_shuffle).
        if !self.order_is_valid() {
            self.reshuffle(current);
        }
        let pos_in_order = current.and_then(|idx| {
            self.order.iter().position(|&i| i == idx)
        });
        let n = self.order.len();
        let next_pos = match (pos_in_order, forward) {
            (Some(p), true) => match self.repeat {
                RepeatMode::All => (p + 1) % n,
                RepeatMode::Off => {
                    if p + 1 < n {
                        p + 1
                    } else {
                        return None;
                    }
                }
            },
            (Some(p), false) => match self.repeat {
                RepeatMode::All => (p + n - 1) % n,
                RepeatMode::Off => p.checked_sub(1)?,
            },
            (None, true) => 0,
            (None, false) => n - 1,
        };
        Some(self.tracks[self.order[next_pos]].clone())
    }

    /// Índice del track actual: el ancla explícita manda; si no, el último
    /// reproducido registrado.
    fn anchor_index(&self, anchor: Option<&str>) -> Option<usize> {
        let key = anchor.or(self.last_played.as_deref())?;
        self.tracks.iter().position(|t| t.identifier() == key)
    }

    /// La permutación cubre exactamente los índices de la cola vigente.
    fn order_is_valid(&self) -> bool {
        self.order.len() == self.tracks.len()
            && self.order.iter().all(|&i| i < self.tracks.len())
    }

    /// Genera una permutación nueva con el track ancla en primera posición.
    fn reshuffle(&mut self, anchor: Option<usize>) {
        let n = self.tracks.len();
        let mut rest: Vec<usize> = (0..n).filter(|i| Some(*i) != anchor).collect();
        // Fisher-Yates con xorshift64 (determinista bajo semilla fija).
        for i in (1..rest.len()).rev() {
            let j = (self.next_random() % (i as u64 + 1)) as usize;
            rest.swap(i, j);
        }
        self.order = match anchor {
            Some(a) if a < n => {
                let mut order = vec![a];
                order.extend(rest);
                order
            }
            _ => rest,
        };
    }

    fn next_random(&mut self) -> u64 {
        // xorshift64star: rápido, periodo completo, sin dependencias.
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source::Source;

    fn track(id: &str) -> Track {
        let mut t = Track::new(id.to_string(), Vec::new(), Source::YouTube);
        t.external_id = Some(id.to_string());
        t
    }

    fn queue(ids: &[&str]) -> QueueManager {
        let mut q = QueueManager::with_seed(42);
        q.set_tracks(ids.iter().map(|id| track(id)).collect());
        q
    }

    fn id(t: &Option<Track>) -> String {
        t.as_ref().unwrap().identifier()
    }

    // ------------------------------------------------- paridad con lo viejo

    #[test]
    fn advances_and_wraps_through_queue() {
        let mut q = queue(&["a", "b", "c"]);
        assert_eq!(id(&q.pick(true, None)), "a", "sin histórico: primera");
        q.mark_played("a");
        assert_eq!(id(&q.pick(true, None)), "b");
        q.mark_played("b");
        assert_eq!(id(&q.pick(true, None)), "c");
        q.mark_played("c");
        assert_eq!(id(&q.pick(true, None)), "a", "envuelve al principio");
    }

    #[test]
    fn backward_goes_previous_and_wraps() {
        let mut q = queue(&["a", "b", "c"]);
        q.mark_played("b");
        assert_eq!(id(&q.pick(false, None)), "a");
        q.mark_played("a");
        assert_eq!(id(&q.pick(false, None)), "c", "desde la primera, a la última");
    }

    #[test]
    fn unknown_anchor_starts_from_extremes() {
        let mut q = queue(&["a", "b", "c"]);
        assert_eq!(id(&q.pick(true, Some("zz"))), "a");
        assert_eq!(id(&q.pick(false, Some("zz"))), "c");
    }

    #[test]
    fn empty_queue_never_picks() {
        let mut q = QueueManager::default();
        assert!(q.pick(true, None).is_none());
        assert!(q.pick(false, None).is_none());
    }

    #[test]
    fn explicit_anchor_wins_over_last_played() {
        let mut q = queue(&["a", "b", "c"]);
        q.mark_played("a");
        assert_eq!(id(&q.pick(true, Some("b"))), "c");
    }

    #[test]
    fn pick_does_not_move_the_anchor() {
        let mut q = queue(&["a", "b", "c"]);
        q.mark_played("a");
        let _ = q.pick(true, None);
        let _ = q.pick(true, None);
        assert_eq!(q.last_played(), Some("a"), "solo mark_played mueve el ancla");
    }

    // -------------------------------------------------------------- repeat

    #[test]
    fn repeat_off_stops_at_edges() {
        let mut q = queue(&["a", "b"]);
        q.set_repeat(RepeatMode::Off);
        q.mark_played("b");
        assert!(q.pick(true, None).is_none(), "sin wrap hacia adelante");
        q.mark_played("a");
        assert!(q.pick(false, None).is_none(), "sin wrap hacia atrás");
    }

    // -------------------------------------------------------------- shuffle

    #[test]
    fn shuffle_keeps_every_track_and_anchor_first() {
        let ids = ["a", "b", "c", "d", "e"];
        let mut q = queue(&ids);
        q.mark_played("c");
        q.set_shuffle(true);

        // Recorre TODA la permutación: mismo conjunto, sin repetidos.
        let mut visited = Vec::new();
        let mut anchor = Some(String::from("c"));
        for _ in 0..ids.len() {
            let next = q.pick(true, anchor.as_deref()).expect("permutación completa");
            visited.push(next.identifier());
            anchor = Some(visited.last().unwrap().clone());
        }
        let mut sorted = visited.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            ["a", "b", "c", "d", "e"],
            "el shuffle no pierde ni duplica tracks"
        );
    }

    #[test]
    fn shuffle_is_deterministic_under_fixed_seed() {
        let build = || {
            let mut q = queue(&["a", "b", "c", "d"]);
            q.mark_played("a");
            q.set_shuffle(true);
            let mut out = Vec::new();
            let mut anchor = Some("a".to_string());
            for _ in 0..4 {
                let t = q.pick(true, anchor.as_deref()).unwrap();
                out.push(t.identifier());
                anchor = Some(out.last().unwrap().clone());
            }
            out
        };
        assert_eq!(build(), build(), "misma semilla, misma permutación");
    }

    #[test]
    fn disabling_shuffle_restores_linear_order() {
        let mut q = queue(&["a", "b", "c"]);
        q.set_shuffle(true);
        q.set_shuffle(false);
        q.mark_played("a");
        assert_eq!(id(&q.pick(true, None)), "b");
    }

    #[test]
    fn replacing_queue_regenerates_shuffle_safely() {
        let mut q = queue(&["a", "b", "c", "d"]);
        q.set_shuffle(true);
        q.set_tracks(vec![track("x"), track("y")]);
        // Navega toda la nueva cola sin colgarse ni perder elementos.
        let mut seen = std::collections::HashSet::new();
        let mut anchor: Option<String> = None;
        for _ in 0..2 {
            let t = q.pick(true, anchor.as_deref()).unwrap();
            seen.insert(t.identifier());
            anchor = Some(t.identifier());
        }
        assert_eq!(seen.len(), 2);
    }
}
