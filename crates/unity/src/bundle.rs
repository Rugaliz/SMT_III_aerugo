//! UnityFS asset bundles (format 6/7, Unity 5.x – 2019.4).

use anyhow::{Context, Result, bail};

use crate::reader::Reader;

#[derive(Debug, Clone)]
pub struct BundleNode {
    pub path: String,
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
}

pub struct Bundle {
    pub format: u32,
    pub unity_version: String,
    pub flags: u32,
    pub nodes: Vec<BundleNode>,
    /// All blocks, decompressed and concatenated.
    pub data: Vec<u8>,
}

const COMPRESSION_MASK: u32 = 0x3F;
const BLOCKS_AND_DIR_COMBINED: u32 = 0x40;
const BLOCKS_INFO_AT_END: u32 = 0x80;
const BLOCK_INFO_NEEDS_PADDING: u32 = 0x200;

pub fn is_bundle(data: &[u8]) -> bool {
    data.starts_with(b"UnityFS\0")
}

fn decompress(kind: u32, src: &[u8], out_len: usize) -> Result<Vec<u8>> {
    match kind {
        0 => Ok(src.to_vec()),
        2 | 3 => {
            let mut out = vec![0u8; out_len];
            let n = lz4_flex::block::decompress_into(src, &mut out).context("lz4")?;
            if n != out_len {
                bail!("lz4 size mismatch {n} != {out_len}");
            }
            Ok(out)
        }
        k => bail!("unsupported compression {k}"),
    }
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle")
            .field("format", &self.format)
            .field("unity_version", &self.unity_version)
            .field("nodes", &self.nodes)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl Bundle {
    pub fn parse(file: &[u8]) -> Result<Self> {
        let mut r = Reader::new(file, true);
        if r.cstr()? != "UnityFS" {
            bail!("not a UnityFS bundle");
        }
        let format = r.u32()?;
        let _player = r.cstr()?;
        let unity_version = r.cstr()?;
        let _size = r.i64()?;
        let ci_size = r.u32()? as usize;
        let ui_size = r.u32()? as usize;
        let flags = r.u32()?;
        if format >= 7 {
            r.align(16)?;
        }
        if flags & BLOCKS_AND_DIR_COMBINED == 0 {
            bail!("separate directory block layout not supported");
        }

        let info_raw =
            if flags & BLOCKS_INFO_AT_END != 0 { &file[file.len() - ci_size..] } else { r.take(ci_size)? };
        let info = decompress(flags & COMPRESSION_MASK, info_raw, ui_size)?;
        if flags & BLOCK_INFO_NEEDS_PADDING != 0 {
            r.align(16)?;
        }

        let mut ir = Reader::new(&info, true);
        ir.take(16)?; // uncompressed data hash
        let block_count = ir.len_i32()?;
        let mut blocks = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            let u = ir.u32()? as usize;
            let c = ir.u32()? as usize;
            let f = ir.u16()? as u32;
            blocks.push((u, c, f));
        }
        let node_count = ir.len_i32()?;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            let offset = ir.u64()?;
            let size = ir.u64()?;
            let flags = ir.u32()?;
            let path = ir.cstr()?;
            nodes.push(BundleNode { path, offset, size, flags });
        }

        let total: usize = blocks.iter().map(|b| b.0).sum();
        let mut data = Vec::with_capacity(total);
        for (u, c, f) in blocks {
            let src = r.take(c)?;
            data.extend_from_slice(&decompress(f & COMPRESSION_MASK, src, u)?);
        }
        Ok(Self { format, unity_version, flags, nodes, data })
    }

    pub fn node_data(&self, n: &BundleNode) -> &[u8] {
        &self.data[n.offset as usize..(n.offset + n.size) as usize]
    }

    /// Bundle nodes that hold SerializedFiles (as opposed to .resS / .resource streams).
    pub fn serialized_nodes(&self) -> impl Iterator<Item = &BundleNode> {
        self.nodes.iter().filter(|n| n.flags & 4 != 0)
    }
}
