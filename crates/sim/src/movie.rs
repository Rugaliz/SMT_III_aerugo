//! The movie manager: the movie table, play and stop requests, and the per-tick process that loads a
//! movie's bundle, starts the player and notices when it ends.
//!
//! The player itself (decoding, sound, display) is the presentation layer's [`MovieBackend`].

use crate::process::{ProcessFn, ProcessHost, ProcessId, Scheduler};

/// Name of the movie process.
pub const PROCESS_NAME: &str = "movieDraw";
/// Priority of the movie process: before the title process.
pub const PRIORITY: u32 = 0x2AFB;

/// One movie of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovieEntry {
    /// Sub-folder under `movie/` (with a trailing slash), or empty.
    pub folder: &'static str,
    pub name: &'static str,
    pub enabled: bool,
    /// Frame rate and vertical sync interval the original asks the engine for while the movie plays.
    pub fps: i32,
    pub vblank: i32,
    /// Asset bundle holding the movie (before the language suffix, for localized movies).
    pub bundle: &'static str,
}

const fn m(
    folder: &'static str,
    name: &'static str,
    enabled: bool,
    fps: i32,
    vblank: i32,
    bundle: &'static str,
) -> MovieEntry {
    MovieEntry { folder, name, enabled, fps, vblank, bundle }
}

/// The movie table, by movie id.
pub const MOVIES: [MovieEntry; 27] = [
    m("", "DDS", false, 30, 2, ""),
    m("", "E500_002", false, 30, 2, ""),
    m("", "E616", true, 60, 1, "m616"),
    m("", "E804", true, 30, 2, "m804"),
    m("", "E804_F", true, 30, 2, "m804"),
    m("", "E804_I", true, 30, 2, "m804"),
    m("", "E658", true, 30, 2, "m658"),
    m("", "G_OVER", true, 30, 2, "mevent"),
    m("", "E673_20", true, 30, 2, "m673"),
    m("", "E674", true, 30, 2, "m674"),
    m("movie_title/", "TITLE1", false, 30, 2, "mtitle"),
    m("", "E617", true, 60, 1, "m617"),
    m("", "E676", true, 30, 2, "m676"),
    m("", "MENU", true, 30, 2, "mtitle"),
    m("", "TITLE2", false, 30, 2, "mtitle"),
    m("", "E688", true, 30, 2, "m688"),
    m("", "E684", true, 30, 2, "m684"),
    m("", "E686", true, 30, 2, "m686"),
    m("", "TITLE3", false, 30, 2, "mtitle"),
    m("", "E727", true, 30, 2, "m727"),
    m("", "E731", true, 30, 2, "m731"),
    m("", "PEEPHOLE", false, 30, 2, "mevent"),
    m("", "E616M", true, 60, 1, "m616m"),
    m("movie_title/", "TITLE1", true, 30, 2, "mtitle"),
    m("", "TITLE2", true, 30, 2, "mtitle"),
    m("movie_title/", "TITLE1M", true, 30, 2, "mtitle"),
    m("", "TITLE2M", true, 30, 2, "mtitle"),
];

/// Language suffixes of localized movies, by language number.
pub const LANGUAGE_SUFFIX: [&str; 8] = ["_ja", "_en", "_fr", "_it", "_de", "_es", "_zh", "_ko"];

/// Movies that exist once per language.
const LOCALIZED: [i32; 3] = [10, 0x17, 0x19];

/// The movie E658, which is paused and released later instead of being stopped at once.
const MOVIE_E658: i32 = 6;

/// Where a requested movie is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovieRequest {
    pub id: i32,
    /// Asset bundle to load.
    pub bundle: String,
    /// Asset path inside the bundle (case-insensitive, as bundle lookups are).
    pub asset: String,
}

impl MovieRequest {
    /// The request for movie `id` in language `lang` (0..8).
    pub fn new(id: i32, lang: usize) -> Option<Self> {
        let e = MOVIES.get(usize::try_from(id).ok()?)?;
        let suffix =
            if LOCALIZED.contains(&id) { LANGUAGE_SUFFIX.get(lang).copied().unwrap_or("_en") } else { "" };
        let bundle = format!("{}{suffix}", e.bundle);
        // The player loads `<dir>/<name>.wmv` from `Assets/AssetBundle/AssetData/`, where `dir` is `movie`
        // or `movie/<folder>`.
        let dir = if e.folder.is_empty() {
            "movie".to_string()
        } else {
            format!("movie/{}", e.folder.trim_end_matches('/'))
        };
        let asset = format!("Assets/AssetBundle/AssetData/{dir}/{}{suffix}.wmv", e.name);
        Some(Self { id, bundle, asset })
    }
}

/// The player, implemented by the presentation layer.
pub trait MovieBackend {
    /// Start loading a bundle.
    fn load_bundle(&mut self, bundle: &str);
    fn bundle_loaded(&self, bundle: &str) -> bool;
    fn unload_bundle(&mut self, bundle: &str);
    /// Create the player for `req` and start it (it prepares, then plays). Failures leave no player.
    fn create(&mut self, req: &MovieRequest);
    /// The player exists and has its first frame and sound ready.
    fn is_prepared(&self) -> bool;
    /// The player is playing (it stops by itself after the last frame).
    fn is_playing(&self) -> bool;
    /// Index of the frame shown now, `None` without a player or before the first frame.
    fn frame(&self) -> Option<i64>;
    /// Remove the player.
    fn release(&mut self);
}

/// Load state of the movie manager.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoadState {
    #[default]
    Idle = 0,
    /// Load the movie's bundle.
    Load = 1,
    /// Wait for the bundle, then start the player.
    Start = 2,
    Playing = 3,
    /// The movie ended; the player is released on the next tick.
    Ended = 4,
    /// E658 is being released after a pause.
    ReleaseLater = 5,
}

/// Movie manager state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MovieManager {
    pub state: LoadState,
    /// Movie being played, -1 for none.
    pub playing: i32,
    /// Ticks of play seen by the end detection.
    step: i32,
    request: Option<MovieRequest>,
    process: Option<ProcessId>,
}

impl MovieManager {
    pub fn new() -> Self {
        Self { playing: -1, ..Self::default() }
    }
}

/// What the movie manager needs from the game.
pub trait MovieHost: ProcessHost + Sized {
    fn movie(&mut self) -> &mut MovieManager;
    fn movie_backend(&mut self) -> &mut dyn MovieBackend;
    /// Current language number (0..8).
    fn language_id(&self) -> usize;
    /// Show or hide the field map and characters (hidden while a movie covers them).
    fn set_field_objects_shown(&mut self, _shown: bool) {}
    /// Remove the sky filter.
    fn sky_filter_delete(&mut self) {}
}

/// Start movie `id`. Returns 1 if a movie is already playing, else 0.
pub fn play<C: MovieHost + 'static>(ctx: &mut C, s: &mut Scheduler<C>, id: i32) -> i32 {
    match ctx.movie().state {
        LoadState::Idle => {
            ctx.movie().playing = id;
            play_file(ctx, s, id);
            0
        }
        LoadState::Playing => 1,
        _ => 0,
    }
}

fn play_file<C: MovieHost + 'static>(ctx: &mut C, s: &mut Scheduler<C>, id: i32) {
    if ctx.movie().process.is_some() {
        log::warn!("duplicate movie request ({id:#x})");
        return;
    }
    let Some(req) = MovieRequest::new(id, ctx.language_id()) else {
        log::error!("no movie {id:#x} in the table");
        return;
    };
    let mm = ctx.movie();
    mm.request = Some(req);
    mm.state = LoadState::Load;
    mm.playing = id;
    mm.process = s.start(PROCESS_NAME, PRIORITY, 1, 1, Some(ProcessFn(process::<C>)), None, None);
}

/// Stop the movie and remove the player.
pub fn stop<C: MovieHost + 'static>(ctx: &mut C, s: &mut Scheduler<C>) {
    let Some(pid) = ctx.movie().process else { return };
    if ctx.movie().playing == MOVIE_E658 {
        log::warn!("stopping movie E658 (paused release) is not ported yet");
    }
    release(ctx);
    s.kill(ctx, pid, 0);
    let mm = ctx.movie();
    mm.process = None;
    mm.state = LoadState::Idle;
    mm.playing = -1;
}

/// 1 when no movie plays (none started, or the last one ended), else 0.
pub fn check_end<C: MovieHost>(ctx: &mut C) -> i32 {
    matches!(ctx.movie().state, LoadState::Idle | LoadState::Ended) as i32
}

/// Frame of the movie shown now, 0 without one.
pub fn frame<C: MovieHost>(ctx: &mut C) -> i32 {
    ctx.movie_backend().frame().map_or(0, |f| f as i32)
}

fn release<C: MovieHost>(ctx: &mut C) {
    let bundle = ctx.movie().request.as_ref().map(|r| r.bundle.clone());
    let b = ctx.movie_backend();
    b.release();
    if let Some(bundle) = bundle {
        b.unload_bundle(&bundle);
    }
    ctx.movie().state = LoadState::Idle;
}

fn process<C: MovieHost>(ctx: &mut C, _: &mut Scheduler<C>, _: ProcessId) -> Option<ProcessFn<C>> {
    match ctx.movie().state {
        LoadState::Load => {
            let bundle = ctx.movie().request.as_ref().map(|r| r.bundle.clone()).unwrap_or_default();
            if !bundle.is_empty() {
                let b = ctx.movie_backend();
                if b.bundle_loaded(&bundle) {
                    b.unload_bundle(&bundle);
                }
                b.load_bundle(&bundle);
            }
            ctx.movie().step = 0;
            ctx.movie().state = LoadState::Start;
        }
        LoadState::Start => {
            let req = ctx.movie().request.clone()?;
            if !req.bundle.is_empty() && !ctx.movie_backend().bundle_loaded(&req.bundle) {
                return None;
            }
            ctx.movie_backend().create(&req);
            ctx.movie().state = LoadState::Playing;
        }
        LoadState::Playing => update(ctx),
        LoadState::Ended => {
            if ctx.movie().playing == MOVIE_E658 {
                log::warn!("movie E658 release after its end is not ported yet");
            }
            release(ctx);
        }
        LoadState::ReleaseLater => log::warn!("movie E658 delayed release is not ported yet"),
        LoadState::Idle => {}
    }
    None
}

/// Per-tick check while playing: notice the end, and hide the field once the movie shows.
fn update<C: MovieHost>(ctx: &mut C) {
    let b = ctx.movie_backend();
    if !b.is_prepared() {
        return;
    }
    if !b.is_playing() {
        let mm = ctx.movie();
        mm.step += 1;
        if mm.step == 1 {
            return;
        }
        ctx.set_field_objects_shown(true);
        ctx.movie().state = LoadState::Ended;
    }
    // The player waits for its first frame before playing, so this runs every tick from here on.
    let mm = ctx.movie();
    mm.step += 1;
    if mm.step == 2 {
        ctx.set_field_objects_shown(false);
        ctx.sky_filter_delete();
    }
    let mm = ctx.movie();
    if mm.step >= 3 {
        mm.step = 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_name_bundle_and_asset() {
        let r = MovieRequest::new(0x17, 1).unwrap();
        assert_eq!(r.bundle, "mtitle_en");
        assert_eq!(r.asset, "Assets/AssetBundle/AssetData/movie/movie_title/TITLE1_en.wmv");
        let r = MovieRequest::new(0x12, 1).unwrap();
        assert_eq!(r.bundle, "mtitle");
        assert_eq!(r.asset, "Assets/AssetBundle/AssetData/movie/TITLE3.wmv");
        assert!(MovieRequest::new(27, 1).is_none());
    }

    #[derive(Default)]
    struct Backend {
        loaded: Vec<String>,
        created: Vec<i32>,
        prepared: bool,
        playing: bool,
        released: u32,
    }

    impl MovieBackend for Backend {
        fn load_bundle(&mut self, bundle: &str) {
            self.loaded.push(bundle.into());
        }
        fn bundle_loaded(&self, bundle: &str) -> bool {
            self.loaded.iter().any(|b| b == bundle)
        }
        fn unload_bundle(&mut self, bundle: &str) {
            self.loaded.retain(|b| b != bundle);
        }
        fn create(&mut self, req: &MovieRequest) {
            self.created.push(req.id);
            self.playing = true;
        }
        fn is_prepared(&self) -> bool {
            self.prepared
        }
        fn is_playing(&self) -> bool {
            self.playing
        }
        fn frame(&self) -> Option<i64> {
            None
        }
        fn release(&mut self) {
            self.released += 1;
            self.playing = false;
        }
    }

    struct Ctx {
        mm: MovieManager,
        backend: Backend,
    }

    impl ProcessHost for Ctx {
        fn process_input(&mut self, _: bool) {}
    }

    impl MovieHost for Ctx {
        fn movie(&mut self) -> &mut MovieManager {
            &mut self.mm
        }
        fn movie_backend(&mut self) -> &mut dyn MovieBackend {
            &mut self.backend
        }
        fn language_id(&self) -> usize {
            1
        }
    }

    #[test]
    fn plays_to_the_end_and_releases() {
        let mut ctx = Ctx { mm: MovieManager::new(), backend: Backend::default() };
        let mut s = Scheduler::new();
        assert_eq!(play(&mut ctx, &mut s, 0x12), 0);
        assert_eq!(check_end(&mut ctx), 0);
        // Second request while one is set up: refused as a duplicate.
        ctx.mm.state = LoadState::Idle;
        play(&mut ctx, &mut s, 0x0D);
        assert_eq!(ctx.mm.request.as_ref().unwrap().id, 0x12);
        ctx.mm.state = LoadState::Load;
        // The process starts one tick late, then loads, then creates the player.
        for _ in 0..3 {
            s.tick(&mut ctx);
        }
        assert_eq!(ctx.backend.created, vec![0x12]);
        assert_eq!(ctx.mm.state, LoadState::Playing);
        ctx.backend.prepared = true;
        s.tick(&mut ctx);
        ctx.backend.playing = false;
        s.tick(&mut ctx);
        assert_eq!(ctx.mm.state, LoadState::Ended);
        assert_eq!(check_end(&mut ctx), 1);
        s.tick(&mut ctx);
        assert_eq!(ctx.backend.released, 1);
        assert!(ctx.backend.loaded.is_empty());
        stop(&mut ctx, &mut s);
        assert_eq!(ctx.mm.playing, -1);
        assert_eq!(ctx.mm.process, None);
    }
}
