//! The game context shared by every process, and the boot sequence.

use aerugo_sim::sequence::{self, SequenceHooks, SequenceHost};
use aerugo_sim::title::{self, Movies, Title, TitleHost, TitleView};
use aerugo_sim::{Fade, FadeHost, Kernel, KernelHost, ProcessHost, Rng};

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

/// Movie playback is not implemented yet: requests are logged.
#[derive(Debug, Default)]
pub struct MovieStub {
    pub playing: Option<i32>,
}

impl Movies for MovieStub {
    fn play(&mut self, id: i32) {
        log::warn!("movie {id:#x}: playback is not implemented yet");
        self.playing = Some(id);
    }
    fn stop(&mut self) {
        self.playing = None;
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
    pub movies: MovieStub,
    pub language: Language,
}

impl Game {
    pub fn new(screens: TitleScreens, language: Language) -> Self {
        Self {
            input_enabled: false,
            rng: Rng::default(),
            fade: Fade::new(),
            title: Title::default(),
            screens,
            movies: MovieStub::default(),
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
    fn movies(&mut self) -> &mut dyn Movies {
        &mut self.movies
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
}
