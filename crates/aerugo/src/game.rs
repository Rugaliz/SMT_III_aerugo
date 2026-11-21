//! The game context shared by every process, and the boot sequence.

use aerugo_sim::movie::{MovieBackend, MovieHost, MovieManager};
use aerugo_sim::sequence::{self, SequenceHooks, SequenceHost};
use aerugo_sim::title::{self, Title, TitleHost, TitleView};
use aerugo_sim::{Fade, FadeHost, Kernel, KernelHost, ProcessHost, Rng};

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
}

impl Game {
    pub fn new(screens: TitleScreens, movies: MoviePlayer, language: Language) -> Self {
        Self {
            input_enabled: false,
            rng: Rng::default(),
            fade: Fade::new(),
            title: Title::default(),
            screens,
            movie: MovieManager::new(),
            movies,
            language,
        }
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
    fn pad_press(&self, _button: u32) -> bool {
        // The mapping from keys and buttons to the game's pad is not ported yet.
        false
    }
    fn language_group(&self) -> i32 {
        self.language.group()
    }
    fn dlc2_installed(&self) -> bool {
        // Downloadable content detection is not implemented: none is installed.
        false
    }
    fn sound_fade_out(&mut self, id: u32) {
        log::debug!("sound fade out {id:#x}: sound is not implemented yet");
    }
    fn play_se(&mut self, id: u32) {
        log::debug!("sound effect {id:#x}: sound effects are not implemented yet");
    }
    fn ui_shown(&self, _layer: i32) -> bool {
        // The kernel's UI display check is not ported: reported as hidden, so steps waiting on it hold.
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
