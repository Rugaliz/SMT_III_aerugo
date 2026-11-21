//! An opened asset bundle: container lookup, object values, resource streams, Texture2D.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::bundle::Bundle;
use crate::serialized::SerializedFile;
use crate::typetree::{ReadOpts, ValueReader};

/// Reference to an object inside a bundle: serialized node index + path ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjRef {
    pub node: usize,
    pub path_id: i64,
}

#[derive(Debug)]
pub struct Archive {
    pub bundle: Bundle,
    /// AssetBundle `m_Container` entries: asset path → object.
    pub container: Vec<(String, ObjRef)>,
}

/// The Unity `TextureFormat` values the game uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    Alpha8,
    Rgb24,
    Rgba32,
    Dxt1,
    Dxt5,
    Bc7,
}

impl TextureFormat {
    pub fn from_unity(id: i64) -> Result<Self> {
        Ok(match id {
            1 => Self::Alpha8,
            3 => Self::Rgb24,
            4 => Self::Rgba32,
            10 => Self::Dxt1,
            12 => Self::Dxt5,
            25 => Self::Bc7,
            _ => bail!("unsupported Unity TextureFormat {id}"),
        })
    }

    /// Byte size of one mip level.
    pub fn level_size(self, w: u32, h: u32) -> usize {
        let blocks = || (w.div_ceil(4) * h.div_ceil(4)) as usize;
        match self {
            Self::Alpha8 => (w * h) as usize,
            Self::Rgb24 => (w * h * 3) as usize,
            Self::Rgba32 => (w * h * 4) as usize,
            Self::Dxt1 => blocks() * 8,
            Self::Dxt5 | Self::Bc7 => blocks() * 16,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Texture2D {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub mip_count: u32,
    /// All mip levels, largest first, rows bottom-up (Unity convention).
    pub data: Vec<u8>,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let bundle = Bundle::parse(&raw)?;
        let mut container = Vec::new();
        for (node, n) in bundle.nodes.iter().enumerate() {
            if n.flags & 4 == 0 {
                continue;
            }
            let sf = SerializedFile::parse(bundle.node_data(n))?;
            for o in &sf.objects {
                if sf.class_name(o) != "AssetBundle" {
                    continue;
                }
                let tree = sf.tree(o).context("AssetBundle without type tree")?;
                let v = ValueReader::new(tree, sf.object_bytes(o)?, sf.big_endian, ReadOpts::default())
                    .read_root()?;
                for c in v["m_Container"].as_array().into_iter().flatten() {
                    if let (Some(p), Some(id)) =
                        (c["first"].as_str(), c["second"]["asset"]["m_PathID"].as_i64())
                    {
                        container.push((p.to_string(), ObjRef { node, path_id: id }));
                    }
                }
            }
        }
        Ok(Self { bundle, container })
    }

    pub fn serialized(&self, node: usize) -> Result<SerializedFile<'_>> {
        SerializedFile::parse(self.bundle.node_data(&self.bundle.nodes[node]))
    }

    /// Find container entries whose path contains `needle`.
    pub fn find(&self, needle: &str) -> Vec<&(String, ObjRef)> {
        self.container.iter().filter(|(p, _)| p.contains(needle)).collect()
    }

    /// Class name and full value of an object, plus its raw bytes (for `$off` blob slicing).
    pub fn read(&self, r: ObjRef, opts: ReadOpts) -> Result<(String, Value, Vec<u8>)> {
        let sf = self.serialized(r.node)?;
        let o = sf.objects.iter().find(|o| o.path_id == r.path_id).context("no such object")?;
        let tree = sf.tree(o).context("object without type tree")?;
        let bytes = sf.object_bytes(o)?;
        let v = ValueReader::new(tree, bytes, sf.big_endian, opts).read_root()?;
        Ok((sf.class_name(o), v, bytes.to_vec()))
    }

    /// Resolve `archive:/CAB-x/CAB-x.resS`-style stream references to node bytes.
    pub fn stream(&self, path: &str, offset: u64, size: u64) -> Result<&[u8]> {
        let file = path.rsplit('/').next().unwrap_or(path);
        let n =
            self.bundle.nodes.iter().find(|n| n.path == file).with_context(|| format!("no node {file}"))?;
        let d = self.bundle.node_data(n);
        let (s, e) = (offset as usize, (offset + size) as usize);
        d.get(s..e).with_context(|| format!("stream {file} {s}..{e} out of range"))
    }

    pub fn texture2d(&self, r: ObjRef) -> Result<Texture2D> {
        let opts = ReadOpts { max_array: usize::MAX, max_bytes: 0, max_string: usize::MAX };
        let (class, v, bytes) = self.read(r, opts)?;
        if class != "Texture2D" {
            bail!("object is a {class}, not Texture2D");
        }
        let int = |k: &str| v[k].as_i64().with_context(|| format!("Texture2D.{k}"));
        let width = int("m_Width")? as u32;
        let height = int("m_Height")? as u32;
        let format = TextureFormat::from_unity(int("m_TextureFormat")?)?;
        let mip_count = int("m_MipCount")? as u32;
        let sd = &v["m_StreamData"];
        let stream_size = sd["size"].as_u64().unwrap_or(0);
        let data = if stream_size > 0 {
            let path = sd["path"].as_str().context("m_StreamData.path")?;
            self.stream(path, sd["offset"].as_u64().unwrap_or(0), stream_size)?.to_vec()
        } else {
            let img = &v["image data"];
            let off = img["$off"].as_u64().context("image data offset")? as usize;
            let len = img["$bytes"].as_u64().context("image data size")? as usize;
            bytes[off..off + len].to_vec()
        };
        let need = format.level_size(width, height);
        if data.len() < need {
            bail!("texture data {} < level 0 size {need}", data.len());
        }
        Ok(Texture2D {
            name: v["m_Name"].as_str().unwrap_or("").to_string(),
            width,
            height,
            format,
            mip_count,
            data,
        })
    }
}
