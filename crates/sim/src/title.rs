//! The title sequence: company logo, opening movie, title screen and main menu.
//!
//! Ported so far: entering and leaving the sequence, the opening steps (optional warning screen, fade from
//! black, the Atlus logo and its fade-out), the opening movies and the press-start movie with its loop back
//! to the logos. The title name drawing over the movies and the main menu are not ported yet; those steps hold.

use crate::fade::FadeHost;
use crate::movie::{self, MovieHost};
use crate::process::{ProcessFn, ProcessId, Scheduler};
use crate::sequence::{CallData, SequenceHooks};

/// Name of the title process.
pub const PROCESS_NAME: &str = "titleProc";
/// Priority of the title process: after the sequence director, before the per-tick fade update.
pub const PRIORITY: u32 = 0x2B19;
/// The title graphics are parked this far above their place while logos and movies play.
pub const PARKED_Y: f32 = 2160.0;
/// Ticks the warning screen stays up before it fades out by itself.
const WARNING_TICKS: i32 = 0x82;
/// Fade length around the warning screen.
const WARNING_FADE: u16 = 30;
/// Ticks of the Atlus logo fade-out.
const LOGO_FADE_TICKS: i32 = 22;
/// Logical pad buttons any of which skips a logo or movie.
pub const SKIP_BUTTONS: [u32; 5] = [1, 3, 2, 0, 12];

/// Language group used by the title: 2 is the Japanese release (shows the warning screen first).
pub const LANGUAGE_JAPANESE: i32 = 2;

/// What the title shows. Implemented by the presentation layer.
pub trait TitleView {
    /// Build the title screens: the menu screen, the Atlus logo and the second logo.
    fn load(&mut self);
    /// Remove everything [`load`](Self::load) built.
    fn unload(&mut self);
    /// Whether the menu screen exists.
    fn menu_loaded(&self) -> bool;
    /// Show or hide the title graphics group (logo, subtitle, copyright, "press start").
    fn set_title_shown(&mut self, shown: bool);
    /// Move the title graphics group.
    fn set_title_position(&mut self, pos: [f32; 3]);
    fn atlus_logo_shown(&self) -> bool;
    fn set_atlus_logo_shown(&mut self, shown: bool);
    /// Tint of the Atlus logo (channels may exceed 1; they are clamped when drawn).
    fn set_atlus_logo_color(&mut self, rgba: [f32; 4]);
    fn rd_logo_shown(&self) -> bool;
    fn set_rd_logo_shown(&mut self, shown: bool);
    fn warning_shown(&self) -> bool;
    fn set_warning_shown(&mut self, shown: bool);
}

/// What the title needs from the game.
pub trait TitleHost: FadeHost + MovieHost + Sized + 'static {
    fn title(&mut self) -> &mut Title;
    fn title_view(&mut self) -> &mut dyn TitleView;
    /// Logical pad button `button` is held.
    fn pad_press(&self, button: u32) -> bool;
    /// Language group of the current language.
    fn language_group(&self) -> i32;
    /// The second downloadable content set is installed.
    fn dlc2_installed(&self) -> bool;
    /// Start fading out the current background music (`id` selects the fade).
    fn sound_fade_out(&mut self, id: u32);
    /// Play sound effect `id`.
    fn play_se(&mut self, id: u32);
    /// Whether UI layer `layer` is being displayed.
    fn ui_shown(&self, layer: i32) -> bool;
}

/// Movie frame after which the alternate opening movie hands over to the title name (step 9).
const OPENING_NAME_FRAME: i32 = 0xA92;
/// Sound effect of confirming on the title screen.
const SE_TITLE_CONFIRM: u32 = 0x3A_0001;
/// Movie ids the title plays.
const MOVIE_PRESS_START: i32 = 0x12;

/// Title state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Title {
    pub step: i32,
    /// Main menu cursor.
    pub pos: i32,
    pub cnt: i32,
    pub loop_flag: i32,
    pub back_alpha: i32,
    pub menu_cnt: i32,
    pub press_fade_in_cnt: i32,
    pub logo_cnt: i32,
    pub mode_pos: i32,
    /// Which opening movie plays: 0 the first, 1 the alternate.
    pub movie_id: i32,
    /// The title graphics group has been set up since the sequence started.
    pub title_set_up: bool,
    /// The warning screen was shown since boot.
    pub warning_done: bool,
    pub warning_cnt: i32,
    pub title2_flag: i32,
    pub title2_mode: i32,
    pub movie_skip: bool,
    /// Counts the opening movie plays; its low bit picks the movie.
    pub opening_count: i32,
    /// Alternates the opening movie when the second content set is installed.
    pub movie_change: i32,
    pub movie_title_init: i32,
    pub start_logo_anim: i32,
    /// Steps not ported yet were reached (reported once).
    unported_reported: bool,
    /// The title name drawing was reached (reported once).
    name_sprite_reported: bool,
}

/// Start hook of the title sequence. `data` may carry an `i32` mode.
pub fn start<C: TitleHost>(ctx: &mut C, s: &mut Scheduler<C>, _back: bool, data: Option<CallData>) -> i32 {
    let mode = match data {
        None => 0,
        Some(d) => *d.downcast_ref::<i32>().expect("title called with data that is not a mode"),
    };
    init(ctx, s, mode);
    1
}

/// End hook: stop the title process and leave at once. The screens are removed by the menu steps that
/// lead out of the title, not here.
pub fn end<C: TitleHost>(ctx: &mut C, s: &mut Scheduler<C>) -> i32 {
    if ctx.title().pos != 8 {
        movie::stop(ctx, s);
    }
    s.kill_by_name(ctx, PROCESS_NAME, 1);
    0
}

/// Check hook: the title never asks to leave by itself.
pub fn check<C: TitleHost>(_: &mut C, _: &mut Scheduler<C>) -> i32 {
    0
}

/// The title sequence's hooks.
pub fn hooks<C: TitleHost>() -> SequenceHooks<C> {
    SequenceHooks { start: Some(start::<C>), end: Some(end::<C>), check: Some(check::<C>) }
}

fn init<C: TitleHost>(ctx: &mut C, s: &mut Scheduler<C>, mode: i32) {
    let t = ctx.title();
    t.step = 0;
    t.pos = 0;
    t.cnt = 0;
    t.press_fade_in_cnt = 0;
    t.logo_cnt = -1;
    t.mode_pos = 0;
    t.movie_id = 0;
    t.menu_cnt = 0;
    t.movie_skip = true;
    t.title_set_up = false;
    if t.title2_mode == 1 {
        t.pos = 2;
    }
    let common = match mode {
        0 => false,
        2 => {
            t.pos = 0;
            true
        }
        3 => {
            t.pos = 1;
            t.title2_flag = 1;
            t.title2_mode = 1;
            true
        }
        _ => true,
    };
    if common {
        ctx.sound_fade_out(0x3A_0000);
        ctx.fade().fade_in_from_current(15);
        movie::stop(ctx, s);
        let t = ctx.title();
        t.back_alpha = 0x80;
        t.cnt = 0;
        t.press_fade_in_cnt = 0;
        t.step = 0x16;
    }
    s.start(PROCESS_NAME, PRIORITY, 0, 0, Some(ProcessFn(run::<C>)), None, None);
    ctx.title_view().load();
}

/// The title name over the movies (logo, subtitle, copyright, "press start"), driven by `logo_cnt`.
/// Not ported yet: the title graphics stay parked.
fn draw_name_sprite<C: TitleHost>(ctx: &mut C) {
    let t = ctx.title();
    if t.logo_cnt == -1 || t.name_sprite_reported {
        return;
    }
    t.name_sprite_reported = true;
    log::warn!("the title name drawing is not ported yet");
}

fn any_skip_pressed<C: TitleHost>(ctx: &C) -> bool {
    SKIP_BUTTONS.iter().any(|&b| ctx.pad_press(b))
}

/// Advance to the next step with the counter reset.
fn next(t: &mut Title) {
    t.cnt = 0;
    t.step += 1;
}

fn run<C: TitleHost>(ctx: &mut C, s: &mut Scheduler<C>, _: ProcessId) -> Option<ProcessFn<C>> {
    if ctx.title_view().menu_loaded() && !ctx.title().title_set_up {
        let v = ctx.title_view();
        v.set_title_shown(true);
        v.set_title_position([0.0, PARKED_Y, 0.0]);
        ctx.title().title_set_up = true;
    }
    match ctx.title().step {
        0 => {
            let lang = ctx.language_group();
            let t = ctx.title();
            next(t);
            if lang == LANGUAGE_JAPANESE && !t.warning_done {
                t.step = 0x24;
                t.warning_done = true;
            }
        }
        1 => next(ctx.title()),
        2 => {
            ctx.fade().fade_in(0, 0, 0, 20);
            next(ctx.title());
            movie::stop(ctx, s);
        }
        3 => {
            let v = ctx.title_view();
            v.set_title_position([0.0, PARKED_Y, 0.0]);
            if !v.atlus_logo_shown() {
                v.set_atlus_logo_shown(true);
            }
            v.set_atlus_logo_color([255.0, 255.0, 255.0, 1.0]);
            if !ctx.fade().is_fading() {
                next(ctx.title());
                movie::stop(ctx, s);
            }
        }
        4 | 7 => {
            if any_skip_pressed(ctx) {
                ctx.title().cnt = 0x32;
            }
            let step = ctx.title().step;
            if ctx.title().cnt < 0x32 {
                ctx.title().cnt += 1;
            } else {
                if step == 7 {
                    ctx.fade().fade_out(0, 0, 0, 20);
                }
                next(ctx.title());
            }
        }
        5 => {
            if ctx.title().cnt < LOGO_FADE_TICKS {
                ctx.title().cnt += 1;
                let a = 1.0 - ctx.title().cnt as f32 / LOGO_FADE_TICKS as f32;
                ctx.title_view().set_atlus_logo_color([255.0, 255.0, 255.0, a]);
            } else {
                let v = ctx.title_view();
                if v.atlus_logo_shown() {
                    v.set_atlus_logo_shown(false);
                }
                let t = ctx.title();
                t.cnt = 0;
                t.step = 8;
            }
        }
        6 => {
            let v = ctx.title_view();
            if !v.rd_logo_shown() {
                v.set_rd_logo_shown(true);
            }
            if ctx.title().cnt < 0x14 {
                ctx.title().cnt += 1;
            } else {
                next(ctx.title());
            }
        }
        8 => {
            if ctx.title().cnt <= 0x15 {
                ctx.title().cnt += 1;
                return None;
            }
            ctx.title().logo_cnt = -1;
            ctx.fade().fade_in(0, 0, 0, 30);
            let dlc2 = ctx.dlc2_installed();
            let t = ctx.title();
            let movie = if t.opening_count & 1 == 0 {
                t.movie_id = 0;
                if dlc2 && t.movie_change != 0 { 0x19 } else { 0x17 }
            } else {
                t.movie_id = 1;
                if !dlc2 {
                    0x18
                } else if t.movie_change == 0 {
                    t.movie_change = 1;
                    0x18
                } else {
                    t.movie_change = 0;
                    0x1A
                }
            };
            movie::play(ctx, s, movie);
            let t = ctx.title();
            t.opening_count += 1;
            next(t);
            let v = ctx.title_view();
            if v.rd_logo_shown() {
                v.set_rd_logo_shown(false);
            }
        }
        9 => {
            // The alternate opening hands over to the title name near its end; the step check below can still
            // move on to 0x18.
            if movie::frame(ctx) > OPENING_NAME_FRAME && ctx.title().movie_id == 1 {
                let t = ctx.title();
                t.logo_cnt = 0;
                t.press_fade_in_cnt = 0;
                t.cnt = 0;
                t.movie_title_init = 0;
                t.step += 1;
            }
            if movie::check_end(ctx) == 0 {
                if !any_skip_pressed(ctx) {
                    return None;
                }
                ctx.fade().fade_out(255, 255, 255, 15);
                let t = ctx.title();
                t.cnt = 0;
                t.movie_title_init = 1;
                t.start_logo_anim = 1;
            } else {
                let t = ctx.title();
                t.cnt = 0;
                t.logo_cnt = -1;
            }
            ctx.title().step = 0x18;
        }
        10 => {
            draw_name_sprite(ctx);
            let ended = movie::check_end(ctx) != 0;
            if !ended {
                if !any_skip_pressed(ctx) {
                    return None;
                }
                ctx.fade().fade_out(0, 0, 0, 15);
                ctx.play_se(SE_TITLE_CONFIRM);
                let t = ctx.title();
                t.cnt = 0;
                t.step = 0xB;
                return None;
            }
            ctx.fade().fade_out(0, 0, 0, 15);
            ctx.title().cnt = 0;
            if ctx.title().movie_id == 0 {
                movie::stop(ctx, s);
                movie::play(ctx, s, MOVIE_PRESS_START);
            }
            ctx.title().step = 0xD;
        }
        0xB | 0x18 => {
            if ctx.fade().is_fading() {
                return None;
            }
            movie::stop(ctx, s);
            next(ctx.title());
        }
        0xD => {
            draw_name_sprite(ctx);
            if ctx.title().cnt > 14 {
                if movie::check_end(ctx) == 0 {
                    return None;
                }
                movie::stop(ctx, s);
                let t = ctx.title();
                t.cnt = 0;
                t.step = 2;
            } else {
                ctx.title().cnt += 1;
            }
        }
        0xE => {
            ctx.fade().fade_in_from_current(15);
            let t = ctx.title();
            t.back_alpha = 0x80;
            t.cnt = 0;
            t.press_fade_in_cnt = 0;
            t.step += 1;
            t.logo_cnt = 0xC0;
            t.press_fade_in_cnt = 0x32;
        }
        0xF => {
            draw_name_sprite(ctx);
            ctx.title().cnt += 1;
            if ctx.fade().is_fading() {
                return None;
            }
            next(ctx.title());
        }
        0x10 => {
            draw_name_sprite(ctx);
            // The original also requires a decoder frame counter of 0 here; nothing ever changes it from 0.
            if movie::check_end(ctx) != 0 {
                let t = ctx.title();
                t.movie_title_init = 0;
                t.start_logo_anim = 0;
                movie::stop(ctx, s);
                ctx.fade().fade_out(0, 0, 0, 15);
                let t = ctx.title();
                t.cnt = 0;
                t.step = 0xD;
                return None;
            }
            ctx.title().cnt += 1;
            if !any_skip_pressed(ctx) || !ctx.ui_shown(0) {
                return None;
            }
            ctx.fade().fade_out(0, 0, 0, 15);
            ctx.play_se(SE_TITLE_CONFIRM);
            let t = ctx.title();
            t.cnt = 0;
            t.step = 0x11;
        }
        0x11 => {
            draw_name_sprite(ctx);
            if ctx.title().cnt == 0x10 {
                movie::stop(ctx, s);
            }
            if ctx.title().cnt > 0x11 {
                let t = ctx.title();
                t.cnt = 0;
                t.step = 0xC;
            } else {
                ctx.title().cnt += 1;
            }
        }
        0x19 => {
            ctx.title().cnt += 1;
            if ctx.title().cnt < 6 {
                return None;
            }
            movie::play(ctx, s, MOVIE_PRESS_START);
            let t = ctx.title();
            t.cnt = 0;
            t.step = 0xE;
        }
        0x24 => {
            let v = ctx.title_view();
            if !v.warning_shown() {
                v.set_warning_shown(true);
            }
            ctx.fade().fade_in(0, 0, 0, WARNING_FADE);
            ctx.title().step = 0x25;
        }
        0x25 => {
            if ctx.fade().is_fading() {
                return None;
            }
            if ctx.title().warning_cnt < WARNING_TICKS {
                ctx.title().warning_cnt += 1;
            } else {
                ctx.fade().fade_out(0, 0, 0, WARNING_FADE);
                ctx.title().step = 0x26;
            }
            if any_skip_pressed(ctx) {
                ctx.fade().fade_out(0, 0, 0, WARNING_FADE);
                ctx.title().step = 0x26;
            }
        }
        0x26 => {
            if ctx.fade().is_fading() {
                return None;
            }
            ctx.title().warning_cnt = 0;
            let v = ctx.title_view();
            if v.warning_shown() {
                v.set_warning_shown(false);
            }
            ctx.title().step = 1;
        }
        step => {
            let t = ctx.title();
            if !t.unported_reported {
                t.unported_reported = true;
                log::warn!("title step {step:#x} is not ported yet; holding");
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fade::Fade;
    use crate::process::ProcessHost;

    #[derive(Default)]
    struct View {
        loaded: bool,
        title_shown: bool,
        title_pos: [f32; 3],
        atlus: bool,
        atlus_color: [f32; 4],
        rd: bool,
        warning: bool,
    }

    impl TitleView for View {
        fn load(&mut self) {
            self.loaded = true;
        }
        fn unload(&mut self) {
            self.loaded = false;
        }
        fn menu_loaded(&self) -> bool {
            self.loaded
        }
        fn set_title_shown(&mut self, shown: bool) {
            self.title_shown = shown;
        }
        fn set_title_position(&mut self, pos: [f32; 3]) {
            self.title_pos = pos;
        }
        fn atlus_logo_shown(&self) -> bool {
            self.atlus
        }
        fn set_atlus_logo_shown(&mut self, shown: bool) {
            self.atlus = shown;
        }
        fn set_atlus_logo_color(&mut self, rgba: [f32; 4]) {
            self.atlus_color = rgba;
        }
        fn rd_logo_shown(&self) -> bool {
            self.rd
        }
        fn set_rd_logo_shown(&mut self, shown: bool) {
            self.rd = shown;
        }
        fn warning_shown(&self) -> bool {
            self.warning
        }
        fn set_warning_shown(&mut self, shown: bool) {
            self.warning = shown;
        }
    }

    #[derive(Default)]
    struct Player {
        played: Vec<i32>,
        playing: bool,
        frame: Option<i64>,
    }

    impl crate::movie::MovieBackend for Player {
        fn load_bundle(&mut self, _: &str) {}
        fn bundle_loaded(&self, _: &str) -> bool {
            true
        }
        fn unload_bundle(&mut self, _: &str) {}
        fn create(&mut self, req: &crate::movie::MovieRequest) {
            self.played.push(req.id);
            self.playing = true;
        }
        fn is_prepared(&self) -> bool {
            true
        }
        fn is_playing(&self) -> bool {
            self.playing
        }
        fn frame(&self) -> Option<i64> {
            self.frame
        }
        fn release(&mut self) {
            self.playing = false;
        }
    }

    #[derive(Default)]
    struct Ctx {
        title: Title,
        fade: Fade,
        view: View,
        movies: Player,
        movie: crate::movie::MovieManager,
        language: i32,
        pressed: bool,
    }

    impl ProcessHost for Ctx {
        fn process_input(&mut self, _: bool) {}
    }

    impl FadeHost for Ctx {
        fn fade(&mut self) -> &mut Fade {
            &mut self.fade
        }
    }

    impl TitleHost for Ctx {
        fn title(&mut self) -> &mut Title {
            &mut self.title
        }
        fn title_view(&mut self) -> &mut dyn TitleView {
            &mut self.view
        }
        fn pad_press(&self, _: u32) -> bool {
            self.pressed
        }
        fn language_group(&self) -> i32 {
            self.language
        }
        fn dlc2_installed(&self) -> bool {
            false
        }
        fn sound_fade_out(&mut self, _: u32) {}
        fn play_se(&mut self, _: u32) {}
        fn ui_shown(&self, _: i32) -> bool {
            true
        }
    }

    impl MovieHost for Ctx {
        fn movie(&mut self) -> &mut crate::movie::MovieManager {
            &mut self.movie
        }
        fn movie_backend(&mut self) -> &mut dyn crate::movie::MovieBackend {
            &mut self.movies
        }
        fn language_id(&self) -> usize {
            1
        }
    }

    /// One tick: the title process, then the drawing process that advances the fade.
    fn tick(ctx: &mut Ctx, s: &mut Scheduler<Ctx>) {
        s.tick(ctx);
    }

    fn boot(language: i32) -> (Ctx, Scheduler<Ctx>) {
        let mut ctx = Ctx { language, movie: crate::movie::MovieManager::new(), ..Default::default() };
        let mut s = Scheduler::new();
        crate::fade::start_draw_process(&mut s);
        start(&mut ctx, &mut s, false, None);
        (ctx, s)
    }

    #[test]
    fn logo_timeline() {
        let (mut ctx, mut s) = boot(1);
        assert!(ctx.view.loaded);
        // Tick 1: set up the title group (parked), step 0 -> 1. Tick 2: 1 -> 2. Tick 3: fade from black starts.
        for _ in 0..3 {
            tick(&mut ctx, &mut s);
        }
        assert!(ctx.view.title_shown);
        assert_eq!(ctx.view.title_pos, [0.0, PARKED_Y, 0.0]);
        assert_eq!(ctx.title.step, 3);
        // Step 3 shows the logo and waits for the 20-tick fade, whose first step ran in tick 3.
        let mut ticks = 0;
        while ctx.title.step == 3 {
            tick(&mut ctx, &mut s);
            ticks += 1;
        }
        assert!(ctx.view.atlus);
        assert_eq!(ctx.view.atlus_color, [255.0, 255.0, 255.0, 1.0]);
        assert_eq!(ticks, 20);
        // Step 4 holds the logo for 50 ticks, step 5 fades it over 22 more.
        let mut ticks = 0;
        while ctx.title.step == 4 {
            tick(&mut ctx, &mut s);
            ticks += 1;
        }
        assert_eq!(ticks, 51);
        tick(&mut ctx, &mut s);
        assert_eq!(ctx.view.atlus_color[3], 1.0 - 1.0 / 22.0);
        while ctx.title.step == 5 {
            tick(&mut ctx, &mut s);
        }
        assert!(!ctx.view.atlus);
        assert_eq!(ctx.title.step, 8);
        // Step 8 waits 22 ticks, then starts the opening movie behind a 30-tick fade in.
        while ctx.title.step == 8 {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.title.movie_id, 0);
        assert_eq!(ctx.title.step, 9);
        // The movie process starts a tick late, loads the bundle, then creates the player.
        for _ in 0..3 {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.movies.played, vec![0x17]);
        // The opening ends: step 0x18 waits for the fade, stops the movie, and after 6 ticks the press-start
        // movie starts.
        ctx.movies.playing = false;
        while ctx.title.step != 0xE {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.movie.playing, 0x12);
        while ctx.title.step != 0x10 {
            tick(&mut ctx, &mut s);
        }
        for _ in 0..3 {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.movies.played, vec![0x17, 0x12]);
        // When it ends the title fades out and starts over from the logos.
        ctx.movies.playing = false;
        while ctx.title.step != 2 {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.movie.playing, -1);
    }

    #[test]
    fn skip_shortens_the_logo_hold() {
        let (mut ctx, mut s) = boot(1);
        while ctx.title.step != 4 {
            tick(&mut ctx, &mut s);
        }
        ctx.pressed = true;
        tick(&mut ctx, &mut s);
        assert_eq!(ctx.title.step, 5);
    }

    #[test]
    fn japanese_shows_the_warning_once() {
        let (mut ctx, mut s) = boot(LANGUAGE_JAPANESE);
        tick(&mut ctx, &mut s);
        assert_eq!(ctx.title.step, 0x24);
        tick(&mut ctx, &mut s);
        assert!(ctx.view.warning);
        while ctx.title.step != 1 {
            tick(&mut ctx, &mut s);
        }
        assert!(!ctx.view.warning);
        assert!(ctx.title.warning_done);
    }
}
