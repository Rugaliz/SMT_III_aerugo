//! The game context shared by every process, and the boot sequence.

use aerugo_sim::input::{self, Check, Keyboard, PadState};
use aerugo_sim::movie::{MovieBackend, MovieHost, MovieManager};
use aerugo_sim::sequence::{self, SequenceHooks, SequenceHost};
use aerugo_sim::title::{self, Title, TitleHost, TitleView};
use aerugo_sim::{Fade, FadeHost, Kernel, KernelHost, ProcessHost, Rng};

use std::cell::RefCell;
use std::rc::Rc;

use aerugo_ui::localize::Localize;

use crate::assets::Assets;
use crate::input::KeyFrame;
use crate::messages;
use crate::movie::MoviePlayer;
use crate::title_view::TitleScreens;

/// Seed of the game's random generator at boot.
const BOOT_SEED: u32 = 0x1234_5678;
/// The sequence entered at boot.
const BOOT_SEQUENCE: i32 = 2;

/// Languages, numbered as the original numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Japanese = 0,
    English = 1,
    French = 2,
    Italian = 3,
    German = 4,
    Spanish = 5,
    Chinese = 6,
    Korean = 7,
}

impl Language {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ja" => Self::Japanese,
            "en" => Self::English,
            "fr" => Self::French,
            "it" => Self::Italian,
            "de" => Self::German,
            "es" => Self::Spanish,
            "zh" => Self::Chinese,
            "ko" => Self::Korean,
            _ => return None,
        })
    }

    /// The language group some screens branch on.
    pub fn group(self) -> i32 {
        match self {
            Self::Japanese => 2,
            Self::Chinese => 4,
            Self::Korean => 3,
            _ => 1,
        }
    }
}

/// State shared by every game process.
#[derive(Debug)]
pub struct Game {
    /// Whether the process running now may read the player's input.
    pub input_enabled: bool,
    pub rng: Rng,
    pub fade: Fade,
    pub title: Title,
    pub screens: TitleScreens,
    pub movie: MovieManager,
    pub movies: MoviePlayer,
    pub language: Language,
    /// The keyboard's configuration and bound-key states.
    pub keyboard: Keyboard,
    /// The gamepad's button states.
    pub pad: PadState,
    /// The keys held this tick.
    pub keys: KeyFrame,
    /// The interface is displayed (the original's UI display flag; starts set).
    pub ui_shown: bool,
    /// The loaded message tables.
    pub messages: Localize,
    assets: Rc<RefCell<Assets>>,
}

impl Game {
    pub fn new(
        assets: Rc<RefCell<Assets>>,
        screens: TitleScreens,
        movies: MoviePlayer,
        language: Language,
        keys: KeyFrame,
    ) -> Self {
        let messages = messages::load_default(&mut assets.borrow_mut(), language as usize);
        Self {
            input_enabled: false,
            rng: Rng::default(),
            fade: Fade::new(),
            title: Title::default(),
            screens,
            movie: MovieManager::new(),
            movies,
            language,
            keyboard: Keyboard::new(),
            pad: PadState::default(),
            keys,
            ui_shown: true,
            messages,
            assets,
        }
    }

    fn pad(&self, button: u32, check: Check) -> bool {
        // The gamepad counts only for processes allowed to read the pad; the keyboard is read directly.
        let pad = self.input_enabled.then_some(&self.pad);
        input::pad_check(pad, &self.keyboard, &self.keys, button, 0, check)
    }

    /// Start the system processes, seed the random generator and enter the first sequence.
    pub fn boot(&mut self, kernel: &mut Kernel<Game>) {
        let s = &mut kernel.scheduler;
        aerugo_sim::fade::start_draw_process(s);
        self.rng.seed(BOOT_SEED);
        sequence::init(s);
        sequence::call(s, BOOT_SEQUENCE, None, 0, false);
    }
}

impl ProcessHost for Game {
    fn process_input(&mut self, enabled: bool) {
        self.input_enabled = enabled;
    }
}

impl KernelHost for Game {}

impl SequenceHost for Game {
    fn sequence(&self, number: i32) -> Option<SequenceHooks<Self>> {
        match number {
            2 => Some(title::hooks()),
            _ => None,
        }
    }
}

impl FadeHost for Game {
    fn fade(&mut self) -> &mut Fade {
        &mut self.fade
    }
}

impl TitleHost for Game {
    fn title(&mut self) -> &mut Title {
        &mut self.title
    }
    fn title_view(&mut self) -> &mut dyn TitleView {
        &mut self.screens
    }
    fn pad_press(&self, button: u32) -> bool {
        self.pad(button, Check::Press)
    }
    fn pad_trigger(&self, button: u32) -> bool {
        self.pad(button, Check::Trigger)
    }
    fn pad_repeat(&self, button: u32) -> bool {
        self.pad(button, Check::Repeat)
    }
    fn localize(&self, id: &str) -> String {
        self.messages.text(id)
    }
    fn load_messages(&mut self, key: &str, filename: &str) {
        messages::load(
            &mut self.assets.borrow_mut(),
            &mut self.messages,
            key,
            filename,
            self.language as usize,
        );
    }
    fn remove_messages(&mut self, name: &str) {
        self.messages.remove(name);
    }
    fn language_group(&self) -> i32 {
        self.language.group()
    }
    fn dlc2_installed(&self) -> bool {
        // Downloadable content detection is not implemented: none is installed.
        false
    }
    fn dlc9_installed(&self) -> bool {
        false
    }
    fn sound_fade_out(&mut self, id: u32) {
        log::debug!("sound fade out {id:#x}: sound is not implemented yet");
    }
    fn play_se(&mut self, id: u32) {
        log::debug!("sound effect {id:#x}: sound effects are not implemented yet");
    }
    fn ui_shown(&self, _layer: i32) -> bool {
        // The interface is shown unless hidden with its toggle (the toggle is not ported yet).
        self.ui_shown
    }
    fn suspend_data_exists(&self) -> bool {
        // Saved games are not read yet.
        false
    }
    fn dlc_message_pending(&mut self) -> bool {
        // Downloadable content detection is not implemented: none is installed.
        false
    }
}

impl MovieHost for Game {
    fn movie(&mut self) -> &mut MovieManager {
        &mut self.movie
    }
    fn movie_backend(&mut self) -> &mut dyn MovieBackend {
        &mut self.movies
    }
    fn language_id(&self) -> usize {
        self.language as usize
    }
}
