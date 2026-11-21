//! The game's sounds: decoding clips from the bundles and the mixer that plays them.

use std::path::Path;
use std::sync::Arc;

use aerugo_audio::{Bank, Decoded, SetupHeaders, SharedMixer, decode};
use anyhow::{Context, Result};

use crate::assets::Assets;

/// A decoded sound and where it loops, if its bank stores loop points.
#[derive(Clone, Debug)]
pub struct Clip {
    pub pcm: Arc<Decoded>,
    /// Loop region in frames (`start..end`). Looping music without one loops over the whole clip.
    pub loop_points: Option<(u64, u64)>,
}

#[derive(Debug)]
pub struct Sound {
    headers: SetupHeaders,
    pub mixer: SharedMixer,
}

impl Sound {
    /// `game` is the installation directory; its player library holds the Vorbis setup headers.
    pub fn new(game: &Path, rate: u32) -> Result<Self> {
        Ok(Self {
            headers: SetupHeaders::load(&game.join("UnityPlayer.dll"))?,
            mixer: SharedMixer::new(rate),
        })
    }

    /// Decode the first sound of a bundle.
    pub fn first_clip(&self, assets: &mut Assets, bundle: &str) -> Result<Clip> {
        let ar = assets.bundle(bundle)?;
        for n in &ar.bundle.nodes {
            let d = ar.bundle.node_data(n);
            if let Some(o) = d.windows(4).position(|w| w == b"FSB5") {
                let bank = Bank::parse(&d[o..])?;
                let s = bank.samples.first().context("empty bank")?;
                let pcm = Arc::new(decode(&bank, s, &self.headers)?);
                let loop_points = s.looping.map(|l| (u64::from(l.start), u64::from(l.end)));
                return Ok(Clip { pcm, loop_points });
            }
        }
        anyhow::bail!("no sound bank in {bundle}")
    }
}
