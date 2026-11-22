//! The game's asset bundles, opened on first use.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aerugo_ui::Prefab;
use aerugo_unity::Archive;
use anyhow::{Context, Result};

#[derive(Debug)]
pub struct Assets {
    game: PathBuf,
    dir: PathBuf,
    bundles: HashMap<String, Archive>,
}

impl Assets {
    /// `game` is the installation directory.
    pub fn new(game: &Path) -> Self {
        Self {
            game: game.to_path_buf(),
            dir: game.join("smt3hd_Data/StreamingAssets/PC"),
            bundles: HashMap::new(),
        }
    }

    /// The installation directory.
    pub fn game(&self) -> &Path {
        &self.game
    }

    pub fn bundle(&mut self, name: &str) -> Result<&Archive> {
        if !self.bundles.contains_key(name) {
            let ar = Archive::open(&self.dir.join(name)).with_context(|| format!("bundle {name}"))?;
            self.bundles.insert(name.to_string(), ar);
        }
        Ok(&self.bundles[name])
    }

    /// The opened bundle holding serialized file `file`, and the file's index in it.
    pub fn opened_file(&self, file: &str) -> Option<(&str, usize)> {
        self.bundles.iter().find_map(|(name, ar)| {
            ar.bundle.nodes.iter().position(|n| n.path == file).map(|node| (name.as_str(), node))
        })
    }

    /// The prefab `name` in folder `dir` of bundle `bundle`.
    pub fn prefab(&mut self, bundle: &str, dir: &str, name: &str) -> Result<Prefab> {
        let path = format!("Assets/AssetBundle/AssetData/{dir}/{name}.prefab");
        let ar = self.bundle(bundle)?;
        Prefab::load(ar, bundle, &path)
    }
}
