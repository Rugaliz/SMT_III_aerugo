//! Aerugo: an engine reimplementation of Shin Megami Tensei III: Nocturne HD Remaster.
//!
//! Usage:
//!   aerugo [--game <install dir>] [--vsync] [view-texture <bundle> <asset path substring>]
//!
//! The installation can also be given with `AERUGO_GAME_DIR`.

use std::path::PathBuf;
use std::time::Instant;

use aerugo_gfx::{Gfx, Sprite, TextureId};
use aerugo_platform::{Platform, PlatformEvent};
use aerugo_sim::FixedTimestep;
use anyhow::{Context, Result, bail};
use sdl3::keyboard::Scancode;

#[derive(Debug)]
struct Args {
    game: Option<PathBuf>,
    vsync: bool,
    view_texture: Option<(String, String)>,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        game: std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from),
        vsync: false,
        view_texture: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--game" => a.game = Some(it.next().context("--game needs a path")?.into()),
            "--vsync" => a.vsync = true,
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
        vec![Sprite { tex: self.tex, dst: [-w, -h, w, h], uv: [0.0, 1.0, 1.0, 0.0], color: [1.0; 4] }]
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = parse_args()?;
    let mut platform = Platform::new("Aerugo: Shin Megami Tensei III", 1280, 720)?;
    let mut gfx = Gfx::new(&platform.window, args.vsync)?;
    log::info!("Vulkan device: {}", gfx.device_name);

    let mut view = None;
    if let Some((bundle, needle)) = &args.view_texture {
        let game = args.game.as_ref().context("view-texture needs --game or AERUGO_GAME_DIR")?;
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
        }
        let sprites = view.as_ref().map(|v| v.sprites(&gfx)).unwrap_or_default();
        gfx.render([0.0, 0.0, 0.0, 1.0], &sprites)?;

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
                clock.total_ticks,
                gfx.present_mode
            );
            frames = 0;
            fps_t = Instant::now();
        }
    }
    Ok(())
}
