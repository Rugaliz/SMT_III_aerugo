//! The player's own asset files (`globalgamemanagers`, `resources.assets`, …): the `Resources` path index and
//! objects read with type trees borrowed from a bundle (the player files are saved without them).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aerugo_unity::resources::{ResourceEntry, resource_container};
use aerugo_unity::{Archive, ReadOpts, SerializedFile, TypeTreeRegistry, ValueReader};
use anyhow::{Context, Result};
use serde_json::Value;

/// The file holding the resource index.
const INDEX_FILE: &str = "globalgamemanagers";

pub struct PlayerFiles {
    dir: PathBuf,
    trees: TypeTreeRegistry,
    files: HashMap<String, Vec<u8>>,
    /// Lower-case `Resources` path → (file name, path id).
    index: HashMap<String, (String, i64)>,
}

impl std::fmt::Debug for PlayerFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerFiles").field("resources", &self.index.len()).finish()
    }
}

/// The last path component (external file references name files by path).
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl PlayerFiles {
    /// Open the player files of the installation `game`; `trees` supplies type trees (a bundle with the classes
    /// to read).
    pub fn open(game: &Path, trees: &Archive) -> Result<Self> {
        let dir = game.join("smt3hd_Data");
        let mut reg = TypeTreeRegistry::new();
        for n in trees.bundle.serialized_nodes() {
            reg.add_file(&SerializedFile::parse(trees.bundle.node_data(n))?);
        }
        let mut me = Self { dir, trees: reg, files: HashMap::new(), index: HashMap::new() };
        let data = me.file(INDEX_FILE)?.to_vec();
        let sf = SerializedFile::parse(&data)?;
        for ResourceEntry { path, file_id, path_id } in resource_container(&sf)? {
            let Some(ext) = (file_id > 0).then(|| sf.externals.get(file_id as usize - 1)).flatten() else {
                continue;
            };
            me.index.insert(path, (file_name(&ext.path).to_string(), path_id));
        }
        Ok(me)
    }

    fn file(&mut self, name: &str) -> Result<&[u8]> {
        if !self.files.contains_key(name) {
            let path = self.dir.join(name);
            let data = std::fs::read(&path).with_context(|| path.display().to_string())?;
            self.files.insert(name.to_string(), data);
        }
        Ok(&self.files[name])
    }

    /// An object's class name and value, and the file's external references (to follow its references).
    pub fn read(&mut self, file: &str, path_id: i64, opts: ReadOpts) -> Result<(String, Value, Vec<String>)> {
        self.file(file)?;
        let data = &self.files[file];
        let mut sf = SerializedFile::parse(data)?;
        self.trees.complete(&mut sf);
        let o = sf
            .objects
            .iter()
            .find(|o| o.path_id == path_id)
            .with_context(|| format!("{file}: no object {path_id}"))?;
        let tree = sf.tree(o).with_context(|| format!("{file}: no type tree for object {path_id}"))?;
        let v = ValueReader::new(tree, sf.object_bytes(o)?, sf.big_endian, opts).read_root()?;
        let externals = sf.externals.iter().map(|e| file_name(&e.path).to_string()).collect();
        Ok((sf.class_name(o), v, externals))
    }

    /// The object a `Resources` path names (case-insensitive): file name and path id.
    pub fn resource(&self, path: &str) -> Option<(String, i64)> {
        self.index.get(&path.to_lowercase()).cloned()
    }
}
