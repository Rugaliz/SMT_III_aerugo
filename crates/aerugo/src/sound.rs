//! The game's sounds: decoding clips from the bundles and the mixer that plays them.

use std::path::Path;
use std::sync::Arc;

use aerugo_audio::{Bank, Decoded, SetupHeaders, SharedMixer, decode};
use anyhow::{Context, Result};

use crate::assets::Assets;

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
    pub fn first_clip(&self, assets: &mut Assets, bundle: &str) -> Result<Arc<Decoded>> {
        let ar = assets.bundle(bundle)?;
        for n in &ar.bundle.nodes {
            let d = ar.bundle.node_data(n);
            if let Some(o) = d.windows(4).position(|w| w == b"FSB5") {
                let bank = Bank::parse(&d[o..])?;
                let s = bank.samples.first().context("empty bank")?;
                return Ok(Arc::new(decode(&bank, s, &self.headers)?));
            }
        }
        anyhow::bail!("no sound bank in {bundle}")
    }
}
