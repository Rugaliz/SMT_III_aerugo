//! The full-screen fade: a colored layer over everything whose opacity steps once per tick.
//!
//! Opacity is kept in 0..=128 (128 = fully covered). A fade *in* uncovers the screen, a fade *out* covers it.
//! The layer keeps its last opacity when a fade ends, so a finished fade-out stays covered.

use crate::process::{ProcessFn, ProcessHost, ProcessId, Scheduler};

/// Name of the per-tick drawing process that advances the fade.
pub const DRAW_PROCESS_NAME: &str = "RequestDraw";
/// Its priority: after the game processes of the tick.
pub const DRAW_PROCESS_PRIORITY: u32 = 20000;

/// Access to the game's fade.
pub trait FadeHost: ProcessHost {
    fn fade(&mut self) -> &mut Fade;
}

/// Start the per-tick drawing process. Of its work only the fade is ported so far.
pub fn start_draw_process<C: FadeHost>(s: &mut Scheduler<C>) {
    s.start(DRAW_PROCESS_NAME, DRAW_PROCESS_PRIORITY, 0, 0, Some(ProcessFn(draw_process::<C>)), None, None);
}

fn draw_process<C: FadeHost>(ctx: &mut C, _: &mut Scheduler<C>, _: ProcessId) -> Option<ProcessFn<C>> {
    ctx.fade().update();
    None
}

const FADING_IN: u32 = 1;
const FADING_OUT: u32 = 2;
const FADING: u32 = FADING_IN | FADING_OUT;

/// Fully covered.
pub const OPAQUE: u8 = 0x80;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fade {
    flags: u32,
    now: u16,
    max: u16,
    rgba: [u8; 4],
    /// While event-script models are loading, fades report as finished. Nothing loads yet in the port.
    pub models_loading: bool,
}

impl Fade {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stop any fade and clear the layer.
    pub fn clear(&mut self) {
        self.flags &= !FADING;
        self.rgba = [0; 4];
    }

    pub fn set_color(&mut self, r: u8, g: u8, b: u8) {
        self.rgba[..3].copy_from_slice(&[r, g, b]);
    }

    /// Uncover the screen from color `r, g, b` over `ticks`. Zero ticks clears the layer at once.
    pub fn fade_in(&mut self, r: u8, g: u8, b: u8, ticks: u16) {
        self.rgba = [r, g, b, OPAQUE];
        self.fade_in_from_current(ticks);
        if ticks == 0 {
            self.rgba[..3].fill(0);
        }
    }

    /// Uncover the screen over `ticks`, keeping the current color.
    pub fn fade_in_from_current(&mut self, ticks: u16) {
        self.now = ticks;
        self.max = ticks;
        if ticks != 0 {
            self.flags = (self.flags | FADING_IN) & !FADING_OUT;
        } else {
            self.now = 0;
            self.max = 0;
            self.rgba[3] = 0;
            self.flags &= !FADING;
        }
    }

    /// Cover the screen with color `r, g, b` over `ticks`. Zero ticks covers it at once.
    pub fn fade_out(&mut self, r: u8, g: u8, b: u8, ticks: u16) {
        self.rgba = [r, g, b, 0];
        self.fade_out_from_current(ticks);
    }

    /// Cover the screen over `ticks`, keeping the current color.
    pub fn fade_out_from_current(&mut self, ticks: u16) {
        self.now = 0;
        self.max = ticks;
        if ticks != 0 {
            self.flags = (self.flags & !FADING_IN) | FADING_OUT;
        } else {
            self.max = 0;
            self.rgba[3] = OPAQUE;
            self.flags &= !FADING;
        }
    }

    /// A fade is in progress.
    pub fn is_fading(&self) -> bool {
        !self.models_loading && self.flags & FADING != 0
    }

    /// Advance one tick.
    pub fn update(&mut self) {
        if !self.is_fading() {
            return;
        }
        if self.flags & FADING == FADING_OUT {
            if u32::from(self.now) == u32::from(self.max).wrapping_sub(1) {
                self.rgba[3] = 0;
            }
            if self.now >= self.max {
                self.stop();
                return;
            }
        } else {
            if self.now == 1 {
                self.rgba[3] = OPAQUE;
            }
            if self.now == 0 {
                self.stop();
                return;
            }
        }
        if self.flags & FADING_IN == 0 {
            self.now = self.now.wrapping_add(1);
        } else {
            self.now = self.now.wrapping_sub(1);
        }
        self.rgba[3] = ((u64::from(self.now) << 7) / u64::from(self.max)) as u8;
        let done_in = self.flags & FADING_IN != 0 && self.now == 0;
        let done_out = self.flags & FADING_OUT != 0 && self.now == self.max;
        if done_in || done_out {
            self.stop();
        }
    }

    fn stop(&mut self) {
        self.now = 0;
        self.max = 0;
        self.flags &= !FADING;
    }

    /// Color and opacity of the layer as drawn this tick (opacity capped at [`OPAQUE`]).
    pub fn layer(&self) -> [u8; 4] {
        let [r, g, b, a] = self.rgba;
        [r, g, b, a.min(OPAQUE)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut Fade) -> Vec<u8> {
        let mut alphas = Vec::new();
        while f.is_fading() {
            f.update();
            alphas.push(f.layer()[3]);
        }
        alphas
    }

    #[test]
    fn fade_in_uncovers_in_steps() {
        let mut f = Fade::new();
        f.fade_in(0, 0, 0, 4);
        assert_eq!(f.layer(), [0, 0, 0, 128]);
        assert_eq!(run(&mut f), vec![96, 64, 32, 0]);
    }

    #[test]
    fn fade_out_covers_and_stays_covered() {
        let mut f = Fade::new();
        f.fade_out(255, 255, 255, 4);
        assert_eq!(run(&mut f), vec![32, 64, 96, 128]);
        f.update();
        assert_eq!(f.layer(), [255, 255, 255, 128]);
    }

    #[test]
    fn zero_length_fades_are_immediate() {
        let mut f = Fade::new();
        f.fade_out(10, 20, 30, 0);
        assert!(!f.is_fading());
        assert_eq!(f.layer(), [10, 20, 30, 128]);
        f.fade_in(10, 20, 30, 0);
        assert_eq!(f.layer(), [0, 0, 0, 0]);
    }

    #[test]
    fn loading_models_hide_the_fade_state() {
        let mut f = Fade::new();
        f.fade_in(0, 0, 0, 2);
        f.models_loading = true;
        assert!(!f.is_fading());
        f.update();
        assert_eq!(f.layer()[3], 128, "no progress while models load");
    }
}
