//! Game logic for Shin Megami Tensei III: Nocturne.
//!
//! The logic runs at a fixed tick ([`tick::GAME_TICK_HZ`]) and is deterministic for given inputs.
//! This crate has no graphics, audio or window dependencies.

pub mod fade;
pub mod kernel;
pub mod process;
pub mod rng;
pub mod sequence;
pub mod sound_ids;
pub mod tick;
pub mod title;

pub use fade::{Fade, FadeHost};
pub use kernel::{Kernel, KernelHost};
pub use process::{FlagScope, ProcessFn, ProcessHost, ProcessId, Scheduler, State};
pub use rng::{LocalTime, Rng};
pub use sequence::{SequenceHooks, SequenceHost};
pub use tick::{FixedTimestep, GAME_TICK_HZ, Steps};
