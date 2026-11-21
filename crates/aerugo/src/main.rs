//! Aerugo: an engine reimplementation of Shin Megami Tensei III: Nocturne HD Remaster.
//!
//! Usage:
//!   aerugo [--game <install dir>] [--vsync] [--lang ja|en|fr|it|de|es|zh|ko]
//!          [view-texture <bundle> <asset path substring>]
//!
//! The installation can also be given with `AERUGO_GAME_DIR`. `AERUGO_CAPTURE=<tick>:<file.ppm>[,...]` saves
//! the frame shown at each listed game tick as a PPM image.

mod assets;
mod game;
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
use crate::title_view::TitleScreens;
use crate::ui_render::UiRenderer;

#[derive(Debug)]
struct Args {
    game: Option<PathBuf>,
    vsync: bool,
    language: Language,
    view_texture: Option<(String, String)>,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        game: std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from),
        vsync: false,
        language: Language::English,
        view_texture: None,
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
    let mut ui_renderer = UiRenderer::new(assets.clone());
    let mut game = Game::new(TitleScreens::new(assets), args.language);
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
        let steps = clock.advance();
        for _ in 0..steps.ticks {
            let _edges = platform.input.latch();
            kernel.tick(&mut game);
        }
        let sprites = match &view {
            Some(v) => v.sprites(&gfx),
            None => ui_renderer.sprites(&mut gfx, &game.screens.ui, game.fade.layer())?,
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
