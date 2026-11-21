//! Game logic for Shin Megami Tensei III: Nocturne.
//!
//! The logic runs at a fixed tick ([`tick::GAME_TICK_HZ`]) and is deterministic for given inputs.
//! This crate has no graphics, audio or window dependencies.

pub mod rng;
pub mod tick;

pub use rng::{LocalTime, Rng};
pub use tick::{FixedTimestep, GAME_TICK_HZ, Steps};
