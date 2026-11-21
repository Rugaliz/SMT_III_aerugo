//! The per-tick driver around the process scheduler: pause and timed stops, frame counters.

use crate::process::{ProcessHost, Scheduler};

/// What the kernel needs from the game around each tick.
pub trait KernelHost: ProcessHost {
    /// Start of a tick, before any process runs (effects bookkeeping, reading the pads).
    fn tick_begin(&mut self) {}
    /// End of a tick.
    fn tick_end(&mut self) {}
}

/// Owns the scheduler and the game's frame counters.
pub struct Kernel<C> {
    pub scheduler: Scheduler<C>,
    /// Ticks in which processes ran (not counted while paused).
    frame_count: u32,
    /// Every tick, paused or not.
    run_count: u32,
    /// Alternates 0/1 each tick in which processes ran; selects double-buffered draw state.
    frame_id: i32,
    paused: bool,
    /// Ticks left in a timed stop (0: stays paused until resumed).
    pause_time: u32,
}

impl<C> std::fmt::Debug for Kernel<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kernel")
            .field("scheduler", &self.scheduler)
            .field("frame_count", &self.frame_count)
            .field("run_count", &self.run_count)
            .field("frame_id", &self.frame_id)
            .field("paused", &self.paused)
            .field("pause_time", &self.pause_time)
            .finish()
    }
}

impl<C> Default for Kernel<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> Kernel<C> {
    pub fn new() -> Self {
        Self {
            scheduler: Scheduler::new(),
            frame_count: 0,
            run_count: 0,
            frame_id: 0,
            paused: false,
            pause_time: 0,
        }
    }

    /// One game tick.
    pub fn tick(&mut self, ctx: &mut C)
    where
        C: KernelHost,
    {
        ctx.tick_begin();
        if !self.paused {
            self.scheduler.tick(ctx);
            self.frame_count = self.frame_count.wrapping_add(1);
            self.frame_id ^= 1;
        } else {
            ctx.process_input(true);
            if self.pause_time != 0 {
                self.pause_time -= 1;
                if self.pause_time == 0 {
                    self.paused = false;
                }
            }
        }
        ctx.tick_end();
        self.run_count = self.run_count.wrapping_add(1);
    }

    /// Stop running processes until [`Kernel::resume`].
    pub fn pause(&mut self) {
        self.paused = true;
        self.pause_time = 0;
    }

    pub fn resume(&mut self) {
        self.paused = false;
        self.pause_time = 0;
    }

    /// Stop running processes for `ticks` ticks.
    pub fn stop_for(&mut self, ticks: u32) {
        self.paused = true;
        self.pause_time = ticks;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    pub fn run_count(&self) -> u32 {
        self.run_count
    }

    pub fn frame_id(&self) -> i32 {
        self.frame_id
    }

    /// Start a game session's counters from zero (as the game does when it boots).
    pub fn reset_frame_count(&mut self) {
        self.frame_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{ProcessFn, ProcessId};

    #[derive(Default)]
    struct Ctx {
        runs: u32,
    }
    impl ProcessHost for Ctx {
        fn process_input(&mut self, _: bool) {}
    }
    impl KernelHost for Ctx {}

    fn count(ctx: &mut Ctx, _: &mut Scheduler<Ctx>, _: ProcessId) -> Option<ProcessFn<Ctx>> {
        ctx.runs += 1;
        None
    }

    #[test]
    fn timed_stop_skips_ticks_then_resumes() {
        let mut k = Kernel::new();
        let mut c = Ctx::default();
        k.scheduler.start("p", 0, 0, 0, Some(ProcessFn(count)), None, None);
        k.tick(&mut c);
        k.stop_for(2);
        k.tick(&mut c);
        k.tick(&mut c);
        assert!(!k.is_paused());
        k.tick(&mut c);
        assert_eq!(c.runs, 2);
        assert_eq!(k.frame_count(), 2);
        assert_eq!(k.run_count(), 4);
        assert_eq!(k.frame_id(), 0);
    }
}
