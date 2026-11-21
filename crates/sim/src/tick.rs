//! Fixed-rate simulation clock.
//!
//! The game's logic counts frames at 30 per second; animations also advance in 1/30 s steps. The
//! simulation runs at exactly that rate whatever the display does, and every displayed frame interpolates
//! by the fraction of the next tick that has already elapsed.

use std::time::Instant;

/// Ticks per second of the game logic.
pub const GAME_TICK_HZ: u64 = 30;

/// At most this many ticks per displayed frame: after a stall the game slows down instead of
/// running a long burst of catch-up ticks.
const MAX_TICKS_PER_FRAME: u32 = 8;

/// One tick in accumulator units (the accumulator counts nanoseconds × [`GAME_TICK_HZ`], so a tick
/// is exactly this many units and the clock never drifts).
const UNIT: u128 = 1_000_000_000;

#[derive(Debug)]
pub struct FixedTimestep {
    acc: u128,
    last: Instant,
    pub total_ticks: u64,
}

/// What to do this frame.
#[derive(Debug, Clone, Copy)]
pub struct Steps {
    /// Simulation ticks to run.
    pub ticks: u32,
    /// Fraction of the next tick that has elapsed, in [0, 1).
    pub alpha: f32,
}

impl Default for FixedTimestep {
    fn default() -> Self {
        Self::new()
    }
}

impl FixedTimestep {
    pub fn new() -> Self {
        Self { acc: 0, last: Instant::now(), total_ticks: 0 }
    }

    /// Account for the real time since the previous call.
    pub fn advance(&mut self) -> Steps {
        let now = Instant::now();
        let dt = now - self.last;
        self.last = now;
        self.add_nanos(dt.as_nanos())
    }

    fn add_nanos(&mut self, nanos: u128) -> Steps {
        self.acc += nanos * GAME_TICK_HZ as u128;
        let mut ticks = 0;
        while self.acc >= UNIT {
            self.acc -= UNIT;
            ticks += 1;
            if ticks == MAX_TICKS_PER_FRAME {
                self.acc %= UNIT;
                break;
            }
        }
        self.total_ticks += ticks as u64;
        Steps { ticks, alpha: (self.acc as f64 / UNIT as f64) as f32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_second_is_exactly_one_seconds_worth_of_ticks() {
        let mut t = FixedTimestep::new();
        let ticks: u32 = (0..1000).map(|_| t.add_nanos(1_000_000).ticks).sum();
        assert_eq!(ticks as u64, GAME_TICK_HZ);
        assert_eq!(t.acc, 0);
    }

    #[test]
    fn stalls_are_capped() {
        let mut t = FixedTimestep::new();
        let s = t.add_nanos(10_000_000_000);
        assert_eq!(s.ticks, MAX_TICKS_PER_FRAME);
    }
}
