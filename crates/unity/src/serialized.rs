//! Unity SerializedFile (versions 9–22) with embedded type trees.

use anyhow::{Result, bail};

use crate::reader::Reader;
use crate::typetree::{TypeTree, read_type_tree};

#[derive(Debug)]
pub struct SerializedType {
    pub class_id: i32,
    pub script_type_index: i16,
    pub tree: Option<TypeTree>,
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub path_id: i64,
    pub byte_start: u64,
    pub byte_size: u32,
    pub type_index: usize,
}

#[derive(Debug, Clone)]
pub struct External {
    pub path: String,
}

#[derive(Debug)]
pub struct SerializedFile<'a> {
    pub data: &'a [u8],
    pub version: u32,
    pub big_endian: bool,
    pub data_offset: u64,
    pub unity_version: String,
    pub platform: i32,
    pub types: Vec<SerializedType>,
    pub objects: Vec<ObjectInfo>,
    pub externals: Vec<External>,
}

/// Quick check for a SerializedFile header (no magic exists; validate the size fields).
pub fn looks_serialized(data: &[u8]) -> bool {
    if data.len() < 20 {
        return false;
    }
    let be = |o: usize| u32::from_be_bytes(data[o..o + 4].try_into().unwrap());
    let (meta, size, ver, off) = (be(0), be(4), be(8), be(12));
    (9..=22).contains(&ver)
        && ((ver >= 22 && meta == 0)
            || (size as usize == data.len() && off as usize <= data.len() && meta < size))
}

impl<'a> SerializedFile<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(data, true);
        let _meta_size = r.u32()?;
        let mut _file_size = r.u32()? as u64;
        let version = r.u32()?;
        let mut data_offset = r.u32()? as u64;
        if !(9..=22).contains(&version) {
            bail!("unsupported SerializedFile version {version}");
        }
        let big_endian = r.u8()? != 0;
        r.take(3)?;
        if version >= 22 {
            let _ = r.u32()?;
            _file_size = r.u64()?;
            data_offset = r.u64()?;
            let _ = r.u64()?;
        }
        r.big_endian = big_endian;

        let unity_version = r.cstr()?;
        let platform = r.i32()?;
        let type_tree_enabled = if version >= 13 { r.u8()? != 0 } else { true };

        let type_count = r.len_i32()?;
        let mut types = Vec::with_capacity(type_count);
        for _ in 0..type_count {
            types.push(read_serialized_type(&mut r, version, type_tree_enabled, false)?);
        }

        if (7..14).contains(&version) {
            let _big_id = r.i32()?;
        }

        let obj_count = r.len_i32()?;
        let mut objects = Vec::with_capacity(obj_count);
        for _ in 0..obj_count {
            if version >= 14 {
                r.align(4)?;
            }
            let path_id = if version >= 14 { r.i64()? } else { r.i32()? as i64 };
            let byte_start = if version >= 22 { r.u64()? } else { r.u32()? as u64 };
            let byte_size = r.u32()?;
            let type_index = r.i32()?;
            if version < 16 {
                let _class = r.u16()?;
            }
            if version < 11 {
                let _destroyed = r.u16()?;
            }
            if (11..17).contains(&version) {
                let _sti = r.i16()?;
            }
            if version == 15 || version == 16 {
                let _stripped = r.u8()?;
            }
            if type_index < 0 || type_index as usize >= types.len() {
                bail!("object {path_id} has bad type index {type_index}");
            }
            objects.push(ObjectInfo { path_id, byte_start, byte_size, type_index: type_index as usize });
        }

        if version >= 11 {
            let n = r.len_i32()?;
            for _ in 0..n {
                let _file_index = r.i32()?;
                if version >= 14 {
                    r.align(4)?;
                    let _ = r.i64()?;
                } else {
                    let _ = r.i32()?;
                }
            }
        }

        let ext_count = r.len_i32()?;
        let mut externals = Vec::with_capacity(ext_count);
        for _ in 0..ext_count {
            let _empty = r.cstr()?;
            r.take(16)?; // guid
            let _kind = r.i32()?;
            externals.push(External { path: r.cstr()? });
        }

        Ok(Self {
            data,
            version,
            big_endian,
            data_offset,
            unity_version,
            platform,
            types,
            objects,
            externals,
        })
    }

    pub fn object_bytes(&self, o: &ObjectInfo) -> Result<&'a [u8]> {
        let s = (self.data_offset + o.byte_start) as usize;
        let e = s + o.byte_size as usize;
        if e > self.data.len() {
            bail!("object {} out of range", o.path_id);
        }
        Ok(&self.data[s..e])
    }

    pub fn tree(&self, o: &ObjectInfo) -> Option<&TypeTree> {
        self.types[o.type_index].tree.as_ref()
    }

    /// Class name from the type tree root, e.g. "Texture2D".
    pub fn class_name(&self, o: &ObjectInfo) -> String {
        match self.tree(o) {
            Some(t) => t.nodes[0].ty.clone(),
            None => format!("Class{}", self.types[o.type_index].class_id),
        }
    }
}

fn read_serialized_type(r: &mut Reader, version: u32, tree: bool, is_ref: bool) -> Result<SerializedType> {
    let class_id = r.i32()?;
    if version >= 16 {
        let _stripped = r.u8()?;
    }
    let script_type_index = if version >= 17 { r.i16()? } else { -1 };
    if version >= 13 {
        let has_script_id = (is_ref && script_type_index >= 0)
            || (version < 16 && class_id < 0)
            || (version >= 16 && class_id == 114);
        if has_script_id {
            r.take(16)?;
        }
        r.take(16)?; // old type hash
    }
    let mut out = SerializedType { class_id, script_type_index, tree: None };
    if tree {
        if version < 12 && version != 10 {
            bail!("legacy type tree format (version {version}) not supported");
        }
        out.tree = Some(read_type_tree(r, version)?);
        if version >= 21 {
            if is_ref {
                r.cstr()?;
                r.cstr()?;
                r.cstr()?;
            } else {
                let n = r.len_i32()?;
                r.take(n * 4)?;
            }
        }
    }
    Ok(out)
}
