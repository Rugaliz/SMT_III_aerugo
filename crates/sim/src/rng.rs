//! The game's random number generator.
//!
//! A shift register over four 32-bit words. Each system that needs random numbers owns its own
//! generator; some are seeded with fixed values, others from the local date and time.

/// Generator state. A new generator is all zeros, and stays at zero until it is seeded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rng {
    pub state: [u32; 4],
}

/// Mixed into the seed when a generator is seeded.
const SEED_MIX: u32 = 0x0AED_1A0C;
/// Mixed into the last state word when a generator is seeded.
const SEED_MIX_LAST: u32 = 0x11BE_81C7;

impl Rng {
    /// A generator seeded with `seed`.
    pub fn seeded(seed: u32) -> Self {
        let mut r = Self::default();
        r.seed(seed);
        r
    }

    /// Next 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        let [s0, s1, s2, s3] = self.state;
        let r = (s3 << 1 | s2 >> 31) ^ (s0 >> 30 | s1 << 2);
        self.state = [r, s0, s1, s2];
        r
    }

    /// A value in [0, 1) from the low 24 bits of the next value.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() & 0x00FF_FFFF) as f32 * (1.0 / 16_777_216.0)
    }

    /// A value in [0, limit). A limit of 0 returns 0 without advancing the generator.
    pub fn below(&mut self, limit: u32) -> u32 {
        if limit == 0 {
            return 0;
        }
        self.next_u32() % limit
    }

    /// Reset the state from a seed.
    pub fn seed(&mut self, seed: u32) {
        let a = seed ^ SEED_MIX;
        let b = a.rotate_right(8);
        let c = (b >> 8) | ((a >> 8) << 24);
        self.state = [a, b, c, (c ^ SEED_MIX_LAST).rotate_right(8)];
    }

    /// Reset the state from the local date and time, as the game does for its unseeded generators.
    pub fn seed_from_time(&mut self, t: &LocalTime) {
        self.seed(time_seed(t));
    }
}

/// Local date and time, field by field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

/// The day offset the game adds for each month: the length of the previous month (February as 29),
/// not a running total.
const MONTH_DAYS: [u16; 13] = [0, 31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// The seed for a moment in time (32-bit wrapping arithmetic).
pub fn time_seed(t: &LocalTime) -> u32 {
    let idx = t.month - 1;
    let idx = if idx < 0 || idx as usize >= MONTH_DAYS.len() { 0 } else { idx as usize };
    let days = (t.day as u32)
        .wrapping_sub(0xB007D)
        .wrapping_add((t.year as u32).wrapping_mul(366))
        .wrapping_add(MONTH_DAYS[idx] as u32);
    let hours = days.wrapping_mul(24).wrapping_add(t.hour as u32);
    let minutes = hours.wrapping_mul(60).wrapping_add(t.minute as u32);
    minutes.wrapping_mul(60).wrapping_add(t.second as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_shifts_the_state() {
        let mut r = Rng { state: [0x8000_0001, 0x4000_0000, 0x8000_0000, 0x0000_0001] };
        // (s3<<1 | s2>>31) = 3, (s0>>30 | s1<<2) = 2, 3 ^ 2 = 1
        assert_eq!(r.next_u32(), 1);
        assert_eq!(r.state, [1, 0x8000_0001, 0x4000_0000, 0x8000_0000]);
    }

    #[test]
    fn unseeded_generator_stays_at_zero() {
        let mut r = Rng::default();
        assert_eq!(r.next_u32(), 0);
        assert_eq!(r.state, [0; 4]);
    }

    #[test]
    fn zero_limit_does_not_advance() {
        let mut r = Rng { state: [1, 2, 3, 4] };
        assert_eq!(r.below(0), 0);
        assert_eq!(r.state, [1, 2, 3, 4]);
    }

    #[test]
    fn seeding_layout() {
        let r = Rng::seeded(123_456);
        let a = 123_456 ^ SEED_MIX;
        assert_eq!(r.state[0], a);
        assert_eq!(r.state[1], a.rotate_right(8));
        assert_eq!(r.state[2], (a.rotate_right(8) >> 8) | ((a >> 8) << 24));
        assert_eq!(r.state[3], (r.state[2] ^ SEED_MIX_LAST).rotate_right(8));
    }

    #[test]
    fn time_seed_counts_seconds() {
        let t = LocalTime { year: 0, month: 1, day: 0xB007D, hour: 1, minute: 2, second: 3 };
        assert_eq!(time_seed(&t), 3600 + 120 + 3);
    }

    #[test]
    fn float_is_in_unit_range() {
        let mut r = Rng::seeded(1);
        for _ in 0..10_000 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f));
        }
    }
}
