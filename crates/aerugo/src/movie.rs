//! The movie player: decodes the requested movie in the background, plays its sound through the mixer's
//! movie group, and paces the pictures by the sound's playback position.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use aerugo_audio::{Decoded, Group, Play, SharedMixer, VoiceId};
use aerugo_gfx::{Gfx, Sprite, VideoId};
use aerugo_media::{MovieStream, Picture};
use aerugo_sim::movie::{MovieBackend, MovieRequest};
use aerugo_unity::ReadOpts;
use anyhow::{Context, Result};

use crate::assets::Assets;

struct Playing {
    id: i32,
    stream: MovieStream,
    voice: Option<VoiceId>,
    audio_start_ms: i64,
    audio_ms: i64,
    /// The sound has played to its end.
    audio_done: bool,
    uploaded: Option<usize>,
    /// Frames the picture was behind the sound, last reported.
    behind_reported: usize,
}

pub struct MoviePlayer {
    assets: Rc<RefCell<Assets>>,
    mixer: SharedMixer,
    loaded: HashSet<String>,
    playing: Option<Playing>,
    target: Option<VideoId>,
}

impl std::fmt::Debug for MoviePlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoviePlayer").field("playing", &self.playing.as_ref().map(|p| p.id)).finish()
    }
}

impl MoviePlayer {
    pub fn new(assets: Rc<RefCell<Assets>>, mixer: SharedMixer) -> Self {
        Self { assets, mixer, loaded: HashSet::new(), playing: None, target: None }
    }

    fn open(&self, req: &MovieRequest) -> Result<MovieStream> {
        let mut assets = self.assets.borrow_mut();
        let ar = assets.bundle(&req.bundle)?;
        let want = req.asset.to_lowercase();
        let (_, r) = ar
            .container
            .iter()
            .find(|(p, _)| p.to_lowercase() == want)
            .with_context(|| format!("no {} in {}", req.asset, req.bundle))?;
        let (class, v, _) = ar.read(*r, ReadOpts::default())?;
        anyhow::ensure!(class == "VideoClip", "{} is a {class}", req.asset);
        let res = &v["m_ExternalResources"];
        let data = ar.stream(
            res["m_Source"].as_str().context("clip source")?,
            res["m_Offset"].as_u64().context("clip offset")?,
            res["m_Size"].as_u64().context("clip size")?,
        )?;
        MovieStream::open(data)
    }

    /// Follow the sound clock. Call once per frame, before the game ticks.
    pub fn update(&mut self) {
        let Some(p) = &mut self.playing else { return };
        if p.voice.is_none()
            && p.stream.prepared()
            && let Some(a) = p.stream.take_audio()
        {
            let pcm = Arc::new(Decoded { rate: a.rate, channels: a.channels as u8, pcm: a.pcm });
            p.audio_start_ms = a.start_ms;
            let play = Play { group: Group::Movie, volume: 1.0, looping: None };
            p.voice = Some(self.mixer.lock().play(pcm, play));
        }
        if let Some(v) = p.voice {
            match self.mixer.lock().time(v) {
                Some(t) => p.audio_ms = p.audio_start_ms + (t * 1000.0) as i64,
                None => p.audio_done = true,
            }
        }
        let t = if p.audio_done { i64::MAX } else { p.audio_ms };
        if p.voice.is_some() {
            p.stream.frame_at(t);
            let due = p.stream.frame_times_ms.partition_point(|&f| f <= t);
            let behind = due.saturating_sub(p.stream.current_index().map_or(0, |i| i + 1));
            if behind > p.behind_reported + 15 {
                log::warn!("movie {:#x}: pictures are {behind} frames behind the sound", p.id);
                p.behind_reported = behind;
            }
        }
        if let Some(e) = p.stream.error() {
            log::error!("movie {:#x}: {e}", p.id);
        }
    }

    /// Upload the current picture if it changed, and return the sprite showing it over `area` (NDC
    /// `x0, y0, x1, y1`), or `None` when no picture is shown.
    pub fn sprite(&mut self, gfx: &mut Gfx, area: [f32; 4]) -> Result<Option<Sprite>> {
        let Some(p) = &mut self.playing else { return Ok(None) };
        let Some(index) = p.stream.current_index() else { return Ok(None) };
        let target = match self.target {
            Some(t) => t,
            None => {
                let (c, d) = (p.stream.coded, p.stream.display);
                let t = gfx.create_video(c, d)?;
                self.target = Some(t);
                t
            }
        };
        if p.uploaded != Some(index) {
            let pic: &Picture = p.stream.frame_at(i64::MIN).map(|(_, pic)| pic).context("current frame")?;
            gfx.upload_video(target, [&pic.y.data, &pic.cb.data, &pic.cr.data])?;
            p.uploaded = Some(index);
        }
        Ok(Some(Sprite::rect(gfx.video_texture(target), area, [0.0, 0.0, 1.0, 1.0], [1.0; 4])))
    }
}

impl MovieBackend for MoviePlayer {
    fn load_bundle(&mut self, bundle: &str) {
        match self.assets.borrow_mut().bundle(bundle) {
            Ok(_) => {
                self.loaded.insert(bundle.to_string());
            }
            Err(e) => log::error!("movie bundle {bundle}: {e:#}"),
        }
    }
    fn bundle_loaded(&self, bundle: &str) -> bool {
        self.loaded.contains(bundle)
    }
    fn unload_bundle(&mut self, bundle: &str) {
        self.loaded.remove(bundle);
    }
    fn create(&mut self, req: &MovieRequest) {
        self.release();
        match self.open(req) {
            Ok(stream) => {
                log::info!("movie {:#x}: {} ({} frames)", req.id, req.asset, stream.frame_times_ms.len());
                self.playing = Some(Playing {
                    id: req.id,
                    stream,
                    voice: None,
                    audio_start_ms: 0,
                    audio_ms: 0,
                    audio_done: false,
                    uploaded: None,
                    behind_reported: 0,
                });
            }
            Err(e) => log::error!("movie {:#x}: {e:#}", req.id),
        }
    }
    fn is_prepared(&self) -> bool {
        self.playing.as_ref().is_some_and(|p| p.voice.is_some())
    }
    fn is_playing(&self) -> bool {
        self.playing.as_ref().is_some_and(|p| !(p.audio_done && p.stream.finished()))
    }
    fn frame(&self) -> Option<i64> {
        self.playing.as_ref().and_then(|p| p.stream.current_index()).map(|i| i as i64)
    }
    fn release(&mut self) {
        if let Some(p) = self.playing.take()
            && let Some(v) = p.voice
        {
            self.mixer.lock().stop(v);
        }
    }
}
