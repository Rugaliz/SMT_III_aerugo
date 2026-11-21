//! Aerugo: an engine reimplementation of Shin Megami Tensei III: Nocturne HD Remaster.
//!
//! Usage:
//!   aerugo [--game <install dir>] [--vsync] [--lang ja|en|fr|it|de|es|zh|ko]
//!          [view-texture <bundle> <asset path substring>] [play-sound <bundle>]
//!
//! The installation can also be given with `AERUGO_GAME_DIR`. `AERUGO_CAPTURE=<tick>:<file.ppm>[,...]` saves
//! the frame shown at each listed game tick as a PPM image.

mod assets;
mod game;
mod movie;
mod sound;
mod title_view;
mod ui_render;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use aerugo_gfx::{Gfx, Sprite, TextureId};
use aerugo_platform::{Platform, PlatformEvent};
use aerugo_sim::{FixedTimestep, Kernel};
use anyhow::{Context, Result, bail};
use sdl3::keyboard::Scancode;

use crate::assets::Assets;
use crate::game::{Game, Language};
use crate::movie::MoviePlayer;
use crate::title_view::TitleScreens;
use crate::ui_render::{UiRenderer, canvas_area};

#[derive(Debug)]
struct Args {
    game: Option<PathBuf>,
    vsync: bool,
    language: Language,
    view_texture: Option<(String, String)>,
    play_sound: Option<String>,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        game: std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from),
        vsync: false,
        language: Language::English,
        view_texture: None,
        play_sound: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--game" => a.game = Some(it.next().context("--game needs a path")?.into()),
            "--vsync" => a.vsync = true,
            "--lang" => {
                let l = it.next().context("--lang needs a language code")?;
                a.language = Language::parse(&l).with_context(|| format!("unknown language {l:?}"))?;
            }
            "play-sound" => a.play_sound = Some(it.next().context("play-sound <bundle>")?),
            "view-texture" => {
                let b = it.next().context("view-texture <bundle> <name>")?;
                let n = it.next().context("view-texture <bundle> <name>")?;
                a.view_texture = Some((b, n));
            }
            _ => bail!("unknown argument {arg}"),
        }
    }
    Ok(a)
}

/// Developer viewer: one texture, centered with its aspect ratio kept.
#[derive(Debug)]
struct TextureView {
    tex: TextureId,
}

impl TextureView {
    fn sprites(&self, gfx: &Gfx) -> Vec<Sprite> {
        let (tw, th) = gfx.texture_size(self.tex);
        let (sw, sh) = gfx.extent();
        let scale = (sw as f32 / tw as f32).min(sh as f32 / th as f32);
        let (w, h) = (tw as f32 * scale / sw as f32, th as f32 * scale / sh as f32);
        // Unity stores texture rows bottom-up.
        vec![Sprite::rect(self.tex, [-w, -h, w, h], [0.0, 1.0, 1.0, 0.0], [1.0; 4])]
    }
}

/// Frames to save: (game tick, output path), in tick order.
fn capture_requests() -> Result<Vec<(u32, PathBuf)>> {
    let Ok(spec) = std::env::var("AERUGO_CAPTURE") else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for item in spec.split(',').filter(|s| !s.is_empty()) {
        let (tick, path) = item.split_once(':').with_context(|| format!("AERUGO_CAPTURE item {item:?}"))?;
        out.push((tick.parse().with_context(|| format!("tick in {item:?}"))?, PathBuf::from(path)));
    }
    out.sort_by_key(|(t, _)| *t);
    Ok(out)
}

fn write_ppm(path: &std::path::Path, c: &aerugo_gfx::Capture) -> Result<()> {
    let mut data = format!("P6\n{} {}\n255\n", c.width, c.height).into_bytes();
    for px in c.rgba.chunks_exact(4) {
        data.extend_from_slice(&px[..3]);
    }
    std::fs::write(path, data).with_context(|| format!("write {}", path.display()))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = parse_args()?;
    let mut platform = Platform::new("Aerugo: Shin Megami Tensei III", 1280, 720)?;
    let mut gfx = Gfx::new(&platform.window, args.vsync)?;
    log::info!("Vulkan device: {}", gfx.device_name);

    let game_dir =
        args.game.clone().context("no game installation: pass --game <dir> or set AERUGO_GAME_DIR")?;
    let mut view = None;
    if let Some((bundle, needle)) = &args.view_texture {
        let game = &game_dir;
        let path = game.join("smt3hd_Data/StreamingAssets/PC").join(bundle);
        let ar = aerugo_unity::Archive::open(&path)?;
        let hit = ar
            .find(needle)
            .into_iter()
            .find(|(_, r)| ar.texture2d(*r).is_ok())
            .with_context(|| format!("no texture matching {needle:?} in {bundle}"))?
            .clone();
        let tex = ar.texture2d(hit.1)?;
        log::info!("{}: {}x{} {:?} mips={}", hit.0, tex.width, tex.height, tex.format, tex.mip_count);
        view = Some(TextureView { tex: gfx.upload_texture(&tex)? });
    }

    let assets = Rc::new(RefCell::new(Assets::new(&game_dir)));
    let sound = sound::Sound::new(&game_dir, aerugo_platform::AUDIO_RATE)?;
    let out = sound.mixer.clone();
    if !platform.set_audio_source(Box::new(move |buf| out.fill(buf))) {
        log::warn!("no audio output");
    }
    if let Some(bundle) = &args.play_sound {
        let clip = sound.first_clip(&mut assets.borrow_mut(), bundle)?;
        let p = &clip.pcm;
        log::info!(
            "{bundle}: {} frames at {} Hz, {} channels, loop {:?}",
            p.frames(),
            p.rate,
            p.channels,
            clip.loop_points
        );
        // Sounds with stored loop points loop between them; others play once.
        let play =
            aerugo_audio::Play { group: aerugo_audio::Group::Bgm, volume: 1.0, looping: clip.loop_points };
        sound.mixer.lock().play(clip.pcm, play);
    }
    let mut ui_renderer = UiRenderer::new(assets.clone());
    let movies = MoviePlayer::new(assets.clone(), sound.mixer.clone());
    let mut game = Game::new(TitleScreens::new(assets), movies, args.language);
    let mut kernel: Kernel<Game> = Kernel::new();
    if view.is_none() {
        game.boot(&mut kernel);
    }
    let mut captures = capture_requests()?;
    let mut capture_path: Option<PathBuf> = None;
    let mut clock = FixedTimestep::new();
    let (mut frames, mut fps_t) = (0u32, Instant::now());
    'main: loop {
        for e in platform.poll() {
            match e {
                PlatformEvent::Quit => break 'main,
                PlatformEvent::Resized(w, h) => gfx.resize(w, h),
                PlatformEvent::Key(Scancode::F11) => platform.toggle_fullscreen(),
                PlatformEvent::Key(Scancode::Escape) if view.is_some() => break 'main,
                PlatformEvent::Key(_) => {}
            }
        }
        game.movies.update();
        let steps = clock.advance();
        for _ in 0..steps.ticks {
            let _edges = platform.input.latch();
            kernel.tick(&mut game);
        }
        let sprites = match &view {
            Some(v) => v.sprites(&gfx),
            None => {
                // The movie is drawn under the whole UI, over the canvas area.
                let a = canvas_area(gfx.extent(), game.screens.ui.canvas);
                let area = [
                    a[0] * 2.0 - 1.0,
                    a[1] * 2.0 - 1.0,
                    (a[0] + a[2]) * 2.0 - 1.0,
                    (a[1] + a[3]) * 2.0 - 1.0,
                ];
                let mut sprites: Vec<Sprite> = game.movies.sprite(&mut gfx, area)?.into_iter().collect();
                sprites.extend(ui_renderer.sprites(&mut gfx, &game.screens.ui, game.fade.layer())?);
                sprites
            }
        };
        if capture_path.is_none() && captures.first().is_some_and(|(t, _)| kernel.frame_count() >= *t) {
            capture_path = Some(captures.remove(0).1);
            gfx.capture_next_frame();
        }
        gfx.render([0.0, 0.0, 0.0, 1.0], &sprites)?;
        if let Some(c) = gfx.take_capture() {
            let path = capture_path.take().expect("a capture was requested");
            write_ppm(&path, &c)?;
            log::info!("saved tick {} to {}", kernel.frame_count(), path.display());
        }

        frames += 1;
        let el = fps_t.elapsed();
        if el.as_secs_f32() >= 1.0 {
            let (w, h) = gfx.extent();
            log::info!(
                "{:.0} fps, {}x{} ({:.3}:1), tick {}, present {:?}",
                frames as f32 / el.as_secs_f32(),
                w,
                h,
                w as f32 / h as f32,
                kernel.frame_count(),
                gfx.present_mode
            );
            frames = 0;
            fps_t = Instant::now();
        }
    }
    Ok(())
}
