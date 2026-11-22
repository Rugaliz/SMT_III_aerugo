//! The title sequence: company logo, opening movie, title screen and main menu.
//!
//! Ported so far: entering and leaving the sequence, the opening steps (optional warning screen, fade from
//! black, the Atlus logo and its fade-out), the opening movies and the press-start movie with its loop back
//! to the logos, with the title name drawn over the movies, and the main menu: its entrance, cursor, help text,
//! cancel back to "press start" and confirm (the chosen entry's actions are not ported yet; they hold).

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
    /// Tint of title sprite `index` (see [`TITLE_SPRITES`]); channels may exceed 1 and are clamped when drawn.
    fn set_title_sprite_color(&mut self, index: usize, rgba: [f32; 4]);
    /// The "press start" animator.
    fn press_anim_enabled(&self) -> bool;
    fn set_press_anim_enabled(&mut self, enabled: bool);
    /// Play the "press start" animator's state `state` from normalized time `t`.
    fn play_press_anim(&mut self, state: &str, t: f32);
    /// The main menu group (entries, cursor, help text; its animator plays the entrance when shown).
    fn menu_shown(&self) -> bool;
    fn set_menu_shown(&mut self, shown: bool);
    /// Main menu entry `n` (1..=10).
    fn menu_item_shown(&self, n: i32) -> bool;
    fn set_menu_item_shown(&mut self, n: i32, shown: bool);
    /// Set entry `n`'s label (rich text).
    fn set_menu_item_text(&mut self, n: i32, text: &str);
    /// World position of entry `n`, if it exists.
    fn menu_item_position(&self, n: i32) -> Option<[f32; 3]>;
    /// Move the menu cursor to a world position.
    fn set_menu_cursor_position(&mut self, pos: [f32; 3]);
    /// Set help line `n` (1..=5, rich text); false if that line does not exist.
    fn set_menu_note_text(&mut self, n: i32, text: &str) -> bool;
    /// The name entry window inside the menu group.
    fn name_window_shown(&self) -> bool;
    fn set_name_window_shown(&mut self, shown: bool);
}

/// The title sprites, in the order the title addresses them (paths below the menu screen).
pub const TITLE_SPRITES: [&str; 7] = [
    "title/title_logo",
    "title/title_Subtitle/Image",
    "title/title_copyright",
    "title/title_press/title_press_base",
    "title/title_press/title_press_base02",
    "title/title_press/titletex_press",
    "title/title_press/titletex_pressg",
];
/// The object carrying the "press start" animator.
pub const PRESS_ANIMATOR: &str = "title/title_press";
/// Animator state started when "press start" appears.
const PRESS_STATE: &str = "title_press";
/// Ticks of the title name fade-in.
const NAME_FADE_TICKS: i32 = 12;
/// `press_fade_in_cnt` at which "press start" appears.
const PRESS_START: i32 = 0x32;
/// Copyright fade: its counter saturates after this many ticks, and its alpha is the 0..128 level over this.
const COPYRIGHT_FADE_TICKS: i32 = 150;
const COPYRIGHT_ALPHA_DIVISOR: f32 = 60.0;

/// What the title needs from the game.
pub trait TitleHost: FadeHost + MovieHost + Sized + 'static {
    fn title(&mut self) -> &mut Title;
    fn title_view(&mut self) -> &mut dyn TitleView;
    /// Logical pad button `button` is held.
    fn pad_press(&self, button: u32) -> bool;
    /// Logical pad button `button` was pressed this tick.
    fn pad_trigger(&self, button: u32) -> bool;
    /// Logical pad button `button` was pressed this tick or is repeating.
    fn pad_repeat(&self, button: u32) -> bool;
    /// The text of message `id` from the loaded message tables.
    fn localize(&self, id: &str) -> String;
    /// Load message table `filename` from the bundle named by `key` (in the current language).
    fn load_messages(&mut self, key: &str, filename: &str);
    /// Remove message table `name`.
    fn remove_messages(&mut self, name: &str);
    /// Language group of the current language.
    fn language_group(&self) -> i32;
    /// The second downloadable content set is installed.
    fn dlc2_installed(&self) -> bool;
    /// The ninth downloadable content set is installed.
    fn dlc9_installed(&self) -> bool;
    /// Start fading out the current background music (`id` selects the fade).
    fn sound_fade_out(&mut self, id: u32);
    /// Play sound effect `id`.
    fn play_se(&mut self, id: u32);
    /// Whether UI layer `layer` is being displayed.
    fn ui_shown(&self, layer: i32) -> bool;
    /// A suspended game (quick save) exists.
    fn suspend_data_exists(&self) -> bool;
    /// Newly installed downloadable content has a message to show.
    fn dlc_message_pending(&mut self) -> bool;
}

/// Movie frame after which the alternate opening movie hands over to the title name (step 9).
const OPENING_NAME_FRAME: i32 = 0xA92;
/// Sound effect of confirming on the title screen.
const SE_TITLE_CONFIRM: u32 = 0x3A_0001;
/// Movie ids the title plays.
const MOVIE_PRESS_START: i32 = 0x12;
const MOVIE_MENU: i32 = 0xD;
/// Main menu cursor with a suspended game (its last entry, of five).
const MENU_CURSOR_SUSPEND: i32 = 5;

/// Main menu entry labels (message ids), by cursor position.
pub const MENU_TEXT: [&str; MENU_ITEMS] = [
    "TSELHELP_L0010",
    "TSELHELP_L0011",
    "TSELHELP_L0022",
    "TSELHELP_L0012",
    "TSELHELP_L0015",
    "TSELHELP_L0013",
    "TSELHELP_L0014",
    "TSELHELP_L0027",
    "TSELHELP_L0029",
    "TSELHELP_L0025",
];
/// Main menu help text (message ids, lines separated by newlines), by cursor position.
pub const MENU_HELP: [&str; MENU_ITEMS] = [
    "TSELHELP_L0004",
    "TSELHELP_L0003",
    "TSELHELP_L0023",
    "TSELHELP_L0007",
    "TSELHELP_L0008",
    "TSELHELP_L0005",
    "TSELHELP_L0006",
    "TSELHELP_L0026",
    "TSELHELP_L0028",
    "TSELHELP_L0024",
];
/// Number of main menu entries.
pub const MENU_ITEMS: usize = 10;
/// Colour materials of the selected entry and of the others (`TMCnn`).
const MENU_COLOR_SELECTED: usize = 0;
const MENU_COLOR: usize = 15;
/// Help lines of the menu in the language group 1, and in the others.
const HELP_LINES_WESTERN: i32 = 5;
const HELP_LINES: i32 = 4;
/// Help line objects in the menu.
const NOTE_LINES: i32 = 5;
/// Ticks of the menu entrance (steps 0x23 and 0x16), and the backdrop alpha step per tick.
const MENU_IN_TICKS: i32 = 0x18;
const MENU_BACK_ALPHA_STEP: i32 = 5;
/// Ticks the cancel step waits before going back to "press start".
const MENU_CANCEL_TICKS: i32 = 0xE;
/// Sound effects of the menu (sound table numbers).
const SE_CURSOR: u32 = 0;
const SE_DECIDE: u32 = 8;
const SE_CANCEL: u32 = 10;
/// Pad buttons of the menu: cancel, confirm (two), up, down.
const PAD_CANCEL: u32 = 3;
const PAD_CONFIRM: [u32; 2] = [1, 0xC];
const PAD_UP: u32 = 6;
const PAD_DOWN: u32 = 7;
/// Fades out of the menu after confirming: the three new game entries, and the others (ticks; the hold that
/// follows).
const CONFIRM_FADE_NEW_GAME: (u16, i32) = (0x3C, 0x2D);
const CONFIRM_FADE: (u16, i32) = (0xF, 4);
/// Fade out when cancelling the menu.
const CANCEL_FADE: u16 = 0xE;
/// Cursor positions with special rules: the continue entry, the exit entry.
const POS_CONTINUE: i32 = 4;
const POS_LICENSE: i32 = 7;
const POS_EXIT: i32 = 9;
/// Message table of the title's dialogs, and its bundle key.
const MESSAGES: &str = "CAMP";
const MESSAGES_BUNDLE: &str = "common";
/// Language number whose menu hides the chronicle entry.
const LANGUAGE_ID_KOREAN: i32 = 7;
/// Step of the name entry, which keeps the name window.
const STEP_NAME_ENTRY: i32 = 0x27;

/// `text` drawn with colour material `n` (`<material="TMCnn">` in front).
pub fn color_tag(text: &str, n: usize) -> String {
    format!("<material=\"TMC{n:02}\">{text}")
}

/// Title state.
#[derive(Clone, Debug, PartialEq)]
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
    pub continue_cursor: i32,
    /// Cursor position + 1 the menu starts on.
    pub menu_cursor_cnt: i32,
    /// Entry number (1-based) the menu cursor was placed on by the last menu drawing.
    pub main_menu_idx_cnt: i32,
    /// Help line counter of the menu drawing (1-based).
    pub main_menu_text_idx_cnt: i32,
    /// 0 when the menu entry labels need redrawing.
    pub title_main_cursor: i32,
    /// The menu drawing has taken its starting cursor.
    pub title_main_init_flag: i32,
    pub title2_pos_flag: bool,
    /// Entry the mouse points at (-1: none). Starts 0.
    pub main_index: i32,
    /// Language number seen by the last menu drawing.
    pub use_lang_cnt: i32,
    /// Hold after the confirm fade.
    pub fadeout_time: i32,
    pub dlc_msg_save: bool,
    /// Steps not ported yet were reached (reported once).
    unported_reported: bool,
}

impl Default for Title {
    /// The state at boot: everything zero except the menu cursor counters, which start at 1.
    fn default() -> Self {
        Self {
            step: 0,
            pos: 0,
            cnt: 0,
            loop_flag: 0,
            back_alpha: 0,
            menu_cnt: 0,
            press_fade_in_cnt: 0,
            logo_cnt: 0,
            mode_pos: 0,
            movie_id: 0,
            title_set_up: false,
            warning_done: false,
            warning_cnt: 0,
            title2_flag: 0,
            title2_mode: 0,
            movie_skip: false,
            opening_count: 0,
            movie_change: 0,
            movie_title_init: 0,
            start_logo_anim: 0,
            continue_cursor: 0,
            menu_cursor_cnt: 1,
            main_menu_idx_cnt: 1,
            main_menu_text_idx_cnt: 1,
            title_main_cursor: 0,
            title_main_init_flag: 0,
            title2_pos_flag: false,
            main_index: 0,
            use_lang_cnt: 0,
            fadeout_time: 0,
            dlc_msg_save: false,
            unported_reported: false,
        }
    }
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
    ctx.remove_messages(MESSAGES);
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
    ctx.load_messages(MESSAGES_BUNDLE, MESSAGES);
    let t = ctx.title();
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

/// The title name over the movies: logo and subtitle fade in over 12 ticks, then "press start" and the
/// copyright appear. Driven by `logo_cnt` (-1: not shown).
fn draw_name_sprite<C: TitleHost>(ctx: &mut C) {
    if ctx.title().logo_cnt == -1 {
        return;
    }
    if ctx.title().start_logo_anim == 0 {
        let v = ctx.title_view();
        if v.press_anim_enabled() {
            v.set_press_anim_enabled(false);
        }
    }
    ctx.title_view().set_title_position([0.0, 0.0, 0.0]);
    if ctx.title().movie_title_init == 0 {
        let v = ctx.title_view();
        v.set_press_anim_enabled(false);
        for i in 0..TITLE_SPRITES.len() {
            v.set_title_sprite_color(i, [255.0, 255.0, 255.0, 0.0]);
        }
        ctx.title().movie_title_init = 1;
    }
    // (The original also steps a sine phase here whose result is never used.)
    let n = ctx.title().logo_cnt.min(NAME_FADE_TICKS);
    let a = ((n << 7) / NAME_FADE_TICKS) as f32 * (1.0 / 128.0);
    let v = ctx.title_view();
    v.set_title_sprite_color(0, [255.0, 255.0, 255.0, a]);
    v.set_title_sprite_color(1, [255.0, 255.0, 255.0, a]);
    if ctx.title().logo_cnt > NAME_FADE_TICKS {
        draw_logo_sprite(ctx);
    }
    ctx.title().logo_cnt += 1;
}

/// "Press start" (animated) and the copyright, from `press_fade_in_cnt` 0x32 on.
fn draw_logo_sprite<C: TitleHost>(ctx: &mut C) {
    ctx.title().press_fade_in_cnt += 1;
    let c = ctx.title().press_fade_in_cnt;
    let v = ctx.title_view();
    if c < PRESS_START {
        v.set_press_anim_enabled(false);
        return;
    }
    v.set_press_anim_enabled(true);
    if c == PRESS_START {
        v.play_press_anim(PRESS_STATE, 0.0);
    }
    let level = ((c - PRESS_START).min(COPYRIGHT_FADE_TICKS) << 7) / COPYRIGHT_FADE_TICKS;
    v.set_title_sprite_color(2, [255.0, 255.0, 255.0, level as f32 / COPYRIGHT_ALPHA_DIVISOR]);
}

/// Keep the menu movie looping: once it ends it is stopped, and on the next call started again.
fn move_play_check<C: TitleHost>(ctx: &mut C, s: &mut Scheduler<C>) {
    if ctx.title().loop_flag != 1 {
        if movie::check_end(ctx) != 0 {
            movie::stop(ctx, s);
            ctx.title().loop_flag = 1;
        }
        return;
    }
    ctx.title().loop_flag = 0;
    movie::play(ctx, s, MOVIE_MENU);
}

/// Draw the main menu: starting cursor, entry labels (the selected one highlighted), which entries exist, the
/// help text of the selected entry and the cursor, placed on the entry selected at the previous call.
fn draw_menu<C: TitleHost>(ctx: &mut C) {
    let help_lines = if ctx.language_group() == 1 { HELP_LINES_WESTERN } else { HELP_LINES };
    // (The original also steps a phase from `menu_cnt` 0xC on, used only by drawing calls that do nothing.)
    let step = ctx.title().step;
    let prev = ctx.title().main_menu_idx_cnt;
    let v = ctx.title_view();
    if step == 0x13 && !v.menu_shown() {
        v.set_menu_shown(true);
    }
    let cursor_pos = v.menu_item_position(prev);
    // (The original also resets the choice window's animator parameter, which nothing on the title sets.)
    if step != STEP_NAME_ENTRY && v.name_window_shown() {
        v.set_name_window_shown(false);
    }
    let suspend = ctx.suspend_data_exists();
    let t = ctx.title();
    if t.title_main_init_flag == 0 {
        if t.title2_flag == 0 || t.title2_pos_flag {
            t.pos = t.menu_cursor_cnt - 1;
        } else {
            t.title2_pos_flag = true;
        }
        t.main_menu_idx_cnt = t.menu_cursor_cnt;
        t.title_main_init_flag = 1;
        if !suspend && t.pos == POS_CONTINUE {
            t.pos = 0;
        }
    }
    // The entry under the mouse takes the cursor (pointer input is not ported: only the initial 0 counts).
    if t.pos != t.main_index && t.main_index != -1 {
        t.pos = t.main_index;
        t.title_main_cursor = 0;
        t.main_index = -1;
        ctx.play_se(SE_CURSOR);
    } else {
        t.main_index = -1;
    }
    if ctx.title().title_main_cursor == 0 {
        let pos = ctx.title().pos;
        for (i, id) in MENU_TEXT.iter().enumerate() {
            let color = if pos == i as i32 { MENU_COLOR_SELECTED } else { MENU_COLOR };
            let text = color_tag(&ctx.localize(id), color);
            ctx.title_view().set_menu_item_text(i as i32 + 1, &text);
        }
        let t = ctx.title();
        t.main_menu_idx_cnt = t.menu_cursor_cnt;
        t.title_main_cursor = 1;
    }
    menu_items_shown(ctx, suspend);
    for n in 1..=NOTE_LINES {
        ctx.title_view().set_menu_note_text(n, "");
    }
    let pos = ctx.title().pos;
    if let Some(id) = usize::try_from(pos).ok().and_then(|p| MENU_HELP.get(p)) {
        let help = ctx.localize(id);
        let lines: Vec<&str> = help.split('\n').collect();
        for i in 0..help_lines {
            let line = lines.get(i as usize).copied().unwrap_or("");
            let n = ctx.title().main_menu_text_idx_cnt;
            if !ctx.title_view().set_menu_note_text(n, &color_tag(line, MENU_COLOR_SELECTED)) {
                break;
            }
            ctx.title().main_menu_text_idx_cnt += 1;
        }
        let t = ctx.title();
        t.main_menu_idx_cnt = pos + 1;
        t.main_menu_text_idx_cnt = 1;
        if let Some(p) = cursor_pos {
            ctx.title_view().set_menu_cursor_position(p);
        }
    }
}

/// Show or hide menu entry `n` (only when it changes).
fn set_item<C: TitleHost>(ctx: &mut C, n: i32, shown: bool) {
    let v = ctx.title_view();
    if v.menu_item_shown(n) != shown {
        v.set_menu_item_shown(n, shown);
    }
}

/// Which main menu entries exist: by mode, installed content, a suspended game and the language.
fn menu_items_shown<C: TitleHost>(ctx: &mut C, suspend: bool) {
    let dlc2 = ctx.dlc2_installed();
    let dlc9 = ctx.dlc9_installed();
    let title2 = ctx.title().title2_flag != 0;
    if !title2 {
        set_item(ctx, 1, true);
        set_item(ctx, 3, false);
        if dlc2 {
            set_item(ctx, 2, true);
        } else {
            set_item(ctx, 4, false);
        }
    } else {
        set_item(ctx, 1, false);
        set_item(ctx, 3, false);
        set_item(ctx, 5, false);
        set_item(ctx, 4, dlc2);
    }
    set_item(ctx, 5, suspend && !title2);
    // (The original has one branch for the Korean language group and one for the others; both do this.)
    if dlc9 && ctx.title().use_lang_cnt != LANGUAGE_ID_KOREAN {
        set_item(ctx, 2, true);
        set_item(ctx, 3, true);
        set_item(ctx, 4, dlc2);
    }
    let lang = ctx.language_id() as i32;
    ctx.title().use_lang_cnt = lang;
    if lang == LANGUAGE_ID_KOREAN {
        set_item(ctx, 3, false);
    }
    // (Pointer colliders for the entries are set up here once; pointer input is not ported.)
}

/// Menu cursor movement: up and down (repeating) wrap around and skip the entries that are not shown.
fn menu_pad_input<C: TitleHost>(ctx: &mut C) {
    let suspend = ctx.suspend_data_exists();
    let dlc2 = ctx.dlc2_installed();
    let dlc9 = ctx.dlc9_installed();
    if ctx.pad_repeat(PAD_UP) {
        let t = ctx.title();
        if t.pos > 0 {
            t.pos -= 1;
        } else {
            t.pos = POS_EXIT;
        }
        ctx.play_se(SE_CURSOR);
        let t = ctx.title();
        if !suspend && t.pos == POS_CONTINUE {
            t.pos = 3;
        }
        if dlc9 && t.use_lang_cnt != LANGUAGE_ID_KOREAN {
            if !dlc2 && t.pos == 3 {
                t.pos = 2;
            }
        } else if dlc2 {
            if t.pos == 2 {
                t.pos = 1;
            }
        } else if t.pos == 2 || t.pos == 3 {
            t.pos = 1;
        }
        t.title_main_cursor = 0;
    }
    if ctx.pad_repeat(PAD_DOWN) {
        ctx.title().pos += 1;
        ctx.play_se(SE_CURSOR);
        let t = ctx.title();
        if dlc9 && t.use_lang_cnt != LANGUAGE_ID_KOREAN {
            if !dlc2 && t.pos == 3 {
                t.pos = 4;
            }
        } else if dlc2 {
            if t.pos == 2 {
                t.pos = 3;
            }
        } else if t.pos == 2 || t.pos == 3 {
            t.pos = 4;
        }
        if !suspend && t.pos == POS_CONTINUE {
            t.pos = 5;
        }
        t.title_main_cursor = 0;
    }
    let t = ctx.title();
    if t.pos < 0 {
        t.pos = POS_EXIT;
    }
    if t.pos >= MENU_ITEMS as i32 {
        t.pos = 0;
    }
}

/// The menu entrance (steps 0x23 and 0x16): draw, show the group, count up while the backdrop fades.
fn menu_in<C: TitleHost>(ctx: &mut C, park_title: bool) {
    ctx.title().menu_cnt += 1;
    draw_menu(ctx);
    let v = ctx.title_view();
    if park_title {
        v.set_title_position([0.0, PARKED_Y, 0.0]);
    }
    if !v.menu_shown() {
        v.set_menu_shown(true);
    }
    let t = ctx.title();
    if t.cnt < MENU_IN_TICKS {
        t.cnt += 1;
        t.back_alpha = (t.back_alpha - MENU_BACK_ALPHA_STEP).max(0);
    } else {
        t.loop_flag = 0;
        t.cnt = 0;
        t.step = 0x13;
    }
}

fn any_skip_pressed<C: TitleHost>(ctx: &C) -> bool {
    SKIP_BUTTONS.iter().any(|&b| ctx.pad_press(b))
}

/// Report once that the title reached something not ported yet (it holds there).
fn report_unported(t: &mut Title, what: &str) {
    if !t.unported_reported {
        t.unported_reported = true;
        log::warn!("{what} is not ported yet; holding");
    }
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
        0xC => {
            if ctx.title().cnt < 4 {
                ctx.title().cnt += 1;
                return None;
            }
            let suspend = ctx.title().title2_flag == 0 && ctx.suspend_data_exists();
            let t = ctx.title();
            t.loop_flag = 0;
            t.menu_cnt = 0;
            t.cnt = 0;
            if t.title2_flag == 0 {
                if suspend && t.continue_cursor == 0 {
                    t.menu_cursor_cnt = MENU_CURSOR_SUSPEND;
                    t.pos = t.menu_cursor_cnt - 1;
                    t.title_main_cursor = 0;
                    t.continue_cursor = 1;
                } else if t.continue_cursor != 0 {
                    t.pos = t.main_menu_idx_cnt - 1;
                } else {
                    t.pos = 0;
                    t.continue_cursor = 1;
                }
            }
            t.step = 0x12;
        }
        0x12 => {
            ctx.title_view().set_title_position([0.0, PARKED_Y, 0.0]);
            movie::play(ctx, s, MOVIE_MENU);
            ctx.fade().fade_in_from_current(15);
            ctx.title().step = 0x1D;
        }
        0x1D => {
            move_play_check(ctx, s);
            if ctx.fade().is_fading() {
                return None;
            }
            // (The field's auto-map and player are removed here; neither exists on the title in the port.)
            if ctx.dlc_message_pending() {
                log::warn!("downloadable content messages are not ported yet");
                return None;
            }
            ctx.title().step = if ctx.title().dlc_msg_save { 0x1F } else { 0x22 };
        }
        0x22 => {
            // (Ends and removes the message window and its text and the shop message tables; none was opened.)
            ctx.title().step = 0x23;
        }
        0x23 => menu_in(ctx, true),
        0x16 => menu_in(ctx, false),
        0x13 => {
            move_play_check(ctx, s);
            // (The button guide is not ported.)
            if ctx.title().title2_flag == 0 {
                menu_pad_input(ctx);
            } else {
                report_unported(ctx.title(), "the second menu mode's cursor");
            }
            draw_menu(ctx);
            if ctx.pad_trigger(PAD_CANCEL) {
                ctx.play_se(SE_CANCEL);
                ctx.fade().fade_out(0, 0, 0, CANCEL_FADE);
                let t = ctx.title();
                t.cnt = 0;
                t.step = 0x1A;
                movie::stop(ctx, s);
                let t = ctx.title();
                t.title2_flag = 0;
                t.title2_mode = 0;
                t.menu_cursor_cnt = 1;
                t.title_main_init_flag = 0;
                t.title_main_cursor = 0;
                t.title2_pos_flag = false;
                let v = ctx.title_view();
                if v.menu_shown() {
                    v.set_menu_shown(false);
                }
                return None;
            }
            // (A pointer click must also be on the selected entry; pointer input is not ported.)
            if !PAD_CONFIRM.iter().any(|&b| ctx.pad_trigger(b)) {
                return None;
            }
            let pos = ctx.title().pos;
            if pos != POS_EXIT {
                let (ticks, hold) = if matches!(pos, 1..=3) { CONFIRM_FADE_NEW_GAME } else { CONFIRM_FADE };
                ctx.fade().fade_out(0, 0, 0, ticks);
                ctx.title().fadeout_time = hold;
                let v = ctx.title_view();
                if v.menu_shown() {
                    v.set_menu_shown(false);
                }
            }
            ctx.play_se(SE_DECIDE);
            next(ctx.title());
        }
        0x14 => {
            if ctx.fade().is_fading() {
                return None;
            }
            let pos = ctx.title().pos;
            if pos != POS_LICENSE && pos != POS_EXIT {
                movie::stop(ctx, s);
            }
            next(ctx.title());
        }
        0x1A => {
            draw_menu(ctx);
            if ctx.title().cnt > MENU_CANCEL_TICKS {
                if movie::check_end(ctx) != 0 {
                    movie::stop(ctx, s);
                }
                let t = ctx.title();
                t.cnt = 0;
                t.step = 0x19;
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
        step => report_unported(ctx.title(), &format!("title step {step:#x}")),
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
        sprite_colors: [[f32; 4]; 7],
        press_anim: bool,
        press_played: Vec<String>,
        menu: bool,
        items: [bool; MENU_ITEMS],
        item_text: [String; MENU_ITEMS],
        notes: [String; 5],
        cursor: [f32; 3],
        name_window: bool,
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
        fn set_title_sprite_color(&mut self, index: usize, rgba: [f32; 4]) {
            self.sprite_colors[index] = rgba;
        }
        fn press_anim_enabled(&self) -> bool {
            self.press_anim
        }
        fn set_press_anim_enabled(&mut self, enabled: bool) {
            self.press_anim = enabled;
        }
        fn play_press_anim(&mut self, state: &str, _: f32) {
            self.press_played.push(state.to_string());
        }
        fn menu_shown(&self) -> bool {
            self.menu
        }
        fn set_menu_shown(&mut self, shown: bool) {
            self.menu = shown;
        }
        fn menu_item_shown(&self, n: i32) -> bool {
            self.items[n as usize - 1]
        }
        fn set_menu_item_shown(&mut self, n: i32, shown: bool) {
            self.items[n as usize - 1] = shown;
        }
        fn set_menu_item_text(&mut self, n: i32, text: &str) {
            self.item_text[n as usize - 1] = text.to_string();
        }
        fn menu_item_position(&self, n: i32) -> Option<[f32; 3]> {
            Some([0.0, -70.0 * n as f32, 0.0])
        }
        fn set_menu_cursor_position(&mut self, pos: [f32; 3]) {
            self.cursor = pos;
        }
        fn set_menu_note_text(&mut self, n: i32, text: &str) -> bool {
            self.notes[n as usize - 1] = text.to_string();
            true
        }
        fn name_window_shown(&self) -> bool {
            self.name_window
        }
        fn set_name_window_shown(&mut self, shown: bool) {
            self.name_window = shown;
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
        /// Buttons pressed this tick (trigger and repeat).
        triggered: Vec<u32>,
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
        fn pad_trigger(&self, button: u32) -> bool {
            self.triggered.contains(&button)
        }
        fn pad_repeat(&self, button: u32) -> bool {
            self.triggered.contains(&button)
        }
        fn localize(&self, id: &str) -> String {
            match id {
                "TSELHELP_L0004" => "line one\nline two".to_string(),
                _ => id.to_string(),
            }
        }
        fn dlc9_installed(&self) -> bool {
            false
        }
        fn load_messages(&mut self, _: &str, _: &str) {}
        fn remove_messages(&mut self, _: &str) {}
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
        fn suspend_data_exists(&self) -> bool {
            false
        }
        fn dlc_message_pending(&mut self) -> bool {
            false
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
        // The title name is unparked and fully shown at once (the counters start past their fades): "press
        // start" is animated without being restarted, and the copyright fades in.
        assert_eq!(ctx.view.title_pos, [0.0, 0.0, 0.0]);
        assert_eq!(ctx.view.sprite_colors[0][3], 1.0);
        assert!(ctx.view.press_anim);
        assert!(ctx.view.press_played.is_empty());
        assert!(ctx.view.sprite_colors[2][3] > 0.0);
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
    fn alternate_opening_fades_the_name_in() {
        let (mut ctx, mut s) = boot(1);
        // Second time round the opening is the alternate movie.
        ctx.title.opening_count = 1;
        while ctx.title.step != 9 {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.title.movie_id, 1);
        for _ in 0..3 {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.movies.played, vec![0x18]);
        // Near its end the title name takes over: fade over 12 ticks, "press start" starts 0x32 counts later.
        ctx.movies.frame = Some(i64::from(OPENING_NAME_FRAME) + 1);
        tick(&mut ctx, &mut s);
        assert_eq!(ctx.title.step, 10);
        for _ in 0..6 {
            tick(&mut ctx, &mut s);
        }
        let a = ctx.view.sprite_colors[0][3];
        assert!(a > 0.0 && a < 1.0, "half-way through the fade: {a}");
        while ctx.view.press_played.is_empty() {
            tick(&mut ctx, &mut s);
            assert!(ctx.title.logo_cnt < 200, "press start never started");
        }
        assert_eq!(ctx.view.press_played, vec!["title_press".to_string()]);
        assert_eq!(ctx.title.press_fade_in_cnt, PRESS_START);
    }

    #[test]
    fn confirm_on_press_start_reaches_the_menu_movie() {
        let (mut ctx, mut s) = boot(1);
        while ctx.title.step != 9 {
            tick(&mut ctx, &mut s);
        }
        // The movie process starts a tick late, loads, then creates the player.
        for _ in 0..3 {
            tick(&mut ctx, &mut s);
        }
        ctx.movies.playing = false;
        while ctx.title.step != 0x10 {
            tick(&mut ctx, &mut s);
        }
        ctx.pressed = true;
        tick(&mut ctx, &mut s);
        ctx.pressed = false;
        assert_eq!(ctx.title.step, 0x11);
        while ctx.title.step != 0x1D {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.title.pos, 0);
        assert_eq!(ctx.title.continue_cursor, 1);
        assert_eq!(ctx.movie.playing, MOVIE_MENU);
        // After the fade the title moves on to the menu drawing (no downloadable content messages).
        while ctx.title.step == 0x1D {
            tick(&mut ctx, &mut s);
        }
        tick(&mut ctx, &mut s);
        assert_eq!(ctx.title.step, 0x23);
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

    /// Boot and confirm on "press start" until the menu entrance starts.
    fn reach_menu() -> (Ctx, Scheduler<Ctx>) {
        let (mut ctx, mut s) = boot(1);
        while ctx.title.step != 9 {
            tick(&mut ctx, &mut s);
        }
        for _ in 0..3 {
            tick(&mut ctx, &mut s);
        }
        ctx.movies.playing = false;
        while ctx.title.step != 0x10 {
            tick(&mut ctx, &mut s);
        }
        ctx.pressed = true;
        tick(&mut ctx, &mut s);
        ctx.pressed = false;
        while ctx.title.step != 0x23 {
            tick(&mut ctx, &mut s);
        }
        (ctx, s)
    }

    fn press(ctx: &mut Ctx, s: &mut Scheduler<Ctx>, button: u32) {
        ctx.triggered = vec![button];
        tick(ctx, s);
        ctx.triggered.clear();
    }

    #[test]
    fn menu_entrance_draws_the_entries() {
        let (mut ctx, mut s) = reach_menu();
        tick(&mut ctx, &mut s);
        assert!(ctx.view.menu);
        assert_eq!(ctx.view.title_pos, [0.0, PARKED_Y, 0.0]);
        // The first entry is highlighted, the others are not; its help text is split into lines.
        assert_eq!(ctx.view.item_text[0], "<material=\"TMC00\">TSELHELP_L0010");
        assert_eq!(ctx.view.item_text[1], "<material=\"TMC15\">TSELHELP_L0011");
        assert_eq!(ctx.view.notes[0], "<material=\"TMC00\">line one");
        assert_eq!(ctx.view.notes[1], "<material=\"TMC00\">line two");
        assert_eq!(ctx.view.notes[2], "<material=\"TMC00\">");
        // No suspended game: no continue entry; no content sets: no chronicle or maniax entries.
        assert!(ctx.view.items[0]);
        assert!(!ctx.view.items[2] && !ctx.view.items[3] && !ctx.view.items[4]);
        let mut ticks = 1;
        while ctx.title.step == 0x23 {
            tick(&mut ctx, &mut s);
            ticks += 1;
        }
        assert_eq!(ticks, MENU_IN_TICKS + 1);
        assert_eq!(ctx.title.step, 0x13);
        assert_eq!(ctx.title.back_alpha, 0x80 - MENU_IN_TICKS * MENU_BACK_ALPHA_STEP);
    }

    #[test]
    fn menu_cursor_skips_missing_entries_and_wraps() {
        let (mut ctx, mut s) = reach_menu();
        while ctx.title.step != 0x13 {
            tick(&mut ctx, &mut s);
        }
        // Down from the first entry: the new game entry, then past the content and continue entries to config.
        press(&mut ctx, &mut s, PAD_DOWN);
        assert_eq!(ctx.title.pos, 1);
        assert_eq!(ctx.view.item_text[1], "<material=\"TMC00\">TSELHELP_L0011");
        assert_eq!(ctx.view.item_text[0], "<material=\"TMC15\">TSELHELP_L0010");
        press(&mut ctx, &mut s, PAD_DOWN);
        assert_eq!(ctx.title.pos, 5);
        press(&mut ctx, &mut s, PAD_UP);
        assert_eq!(ctx.title.pos, 1);
        // Up from the first entry wraps to exit.
        press(&mut ctx, &mut s, PAD_UP);
        press(&mut ctx, &mut s, PAD_UP);
        assert_eq!(ctx.title.pos, POS_EXIT);
        // The cursor follows one call late: it sits on the entry selected before this tick.
        tick(&mut ctx, &mut s);
        assert_eq!(ctx.view.cursor, [0.0, -70.0 * 10.0, 0.0]);
    }

    #[test]
    fn menu_cancel_goes_back_to_press_start() {
        let (mut ctx, mut s) = reach_menu();
        while ctx.title.step != 0x13 {
            tick(&mut ctx, &mut s);
        }
        press(&mut ctx, &mut s, PAD_CANCEL);
        assert_eq!(ctx.title.step, 0x1A);
        assert!(!ctx.view.menu);
        assert_eq!(ctx.title.menu_cursor_cnt, 1);
        let mut ticks = 0;
        while ctx.title.step == 0x1A {
            tick(&mut ctx, &mut s);
            ticks += 1;
        }
        assert_eq!(ticks, MENU_CANCEL_TICKS + 2);
        while ctx.title.step != 0xE {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.movie.playing, MOVIE_PRESS_START);
    }

    #[test]
    fn menu_confirm_fades_out() {
        let (mut ctx, mut s) = reach_menu();
        while ctx.title.step != 0x13 {
            tick(&mut ctx, &mut s);
        }
        press(&mut ctx, &mut s, PAD_DOWN);
        press(&mut ctx, &mut s, PAD_CONFIRM[0]);
        assert_eq!(ctx.title.step, 0x14);
        assert_eq!(ctx.title.fadeout_time, CONFIRM_FADE_NEW_GAME.1);
        assert!(!ctx.view.menu);
        while ctx.title.step == 0x14 {
            tick(&mut ctx, &mut s);
        }
        assert_eq!(ctx.title.step, 0x15);
    }
}
