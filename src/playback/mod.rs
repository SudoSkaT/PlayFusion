//! Capa de Playback (aplicación): componentes de orquestación sobre el motor.
//!
//! El contrato del motor ([`crate::app::audio::PlaybackEngine`]) y su router
//! ([`crate::app::playback::PlaybackRouter`]) ya existían y se reutilizan tal
//! cual (spec §16). Este módulo añade las piezas que faltaban:
//!
//! | Módulo           | Responsabilidad                                     |
//! |------------------|-----------------------------------------------------|
//! | [`position_clock`] | reloj maestro derivado del audio (spec §17)       |
//! | [`queue`]        | cola formal: next/prev/wrap/shuffle/repeat          |
//! | [`preload`]      | preparación SOLO del siguiente track (spec §36)     |
//! | [`recovery`]     | clasificación y recuperación acotada (spec §37)     |

pub mod position_clock;
pub mod preload;
pub mod queue;
pub mod recovery;

#[cfg(test)]
pub mod karaoke_tests;

pub use position_clock::{ClockEvent, PositionClock};
pub use preload::{PreloadConfig, PreloadManager, should_preload};
pub use queue::{QueueManager, RepeatMode};
pub use recovery::{RecoveryBudget, RecoveryAction, decide_recovery};
