//! Type trees (blob format, SerializedFile v12+) and a generic typetree-driven value reader.

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use crate::reader::Reader;

#[derive(Debug, Clone)]
pub struct TypeNode {
    pub ty: String,
    pub name: String,
    pub byte_size: i32,
    pub level: u8,
    pub type_flags: u8,
    pub meta_flag: i32,
    pub children: Vec<usize>,
}

impl TypeNode {
    fn is_array(&self) -> bool {
        self.type_flags & 1 != 0
    }
    fn aligned(&self) -> bool {
        self.meta_flag & 0x4000 != 0
    }
}

#[derive(Debug, Clone)]
pub struct TypeTree {
    pub nodes: Vec<TypeNode>,
}

/// Unity's built-in string table referenced by offsets with the high bit set.
/// Order matters: an entry's offset is the byte position in the NUL-joined list.
const COMMON_STRINGS: &[&str] = &[
    "AABB",
    "AnimationClip",
    "AnimationCurve",
    "AnimationState",
    "Array",
    "Base",
    "BitField",
    "bitset",
    "bool",
    "char",
    "ColorRGBA",
    "Component",
    "data",
    "deque",
    "double",
    "dynamic_array",
    "FastPropertyName",
    "first",
    "float",
    "Font",
    "GameObject",
    "Generic Mono",
    "GradientNEW",
    "GUID",
    "GUIStyle",
    "int",
    "list",
    "long long",
    "map",
    "Matrix4x4f",
    "MdFour",
    "MonoBehaviour",
    "MonoScript",
    "m_ByteSize",
    "m_Curve",
    "m_EditorClassIdentifier",
    "m_EditorHideFlags",
    "m_Enabled",
    "m_ExtensionPtr",
    "m_GameObject",
    "m_Index",
    "m_IsArray",
    "m_IsStatic",
    "m_MetaFlag",
    "m_Name",
    "m_ObjectHideFlags",
    "m_PrefabInternal",
    "m_PrefabParentObject",
    "m_Script",
    "m_StaticEditorFlags",
    "m_Type",
    "m_Version",
    "Object",
    "pair",
    "PPtr<Component>",
    "PPtr<GameObject>",
    "PPtr<Material>",
    "PPtr<MonoBehaviour>",
    "PPtr<MonoScript>",
    "PPtr<Object>",
    "PPtr<Prefab>",
    "PPtr<Sprite>",
    "PPtr<TextAsset>",
    "PPtr<Texture>",
    "PPtr<Texture2D>",
    "PPtr<Transform>",
    "Prefab",
    "Quaternionf",
    "Rectf",
    "RectInt",
    "RectOffset",
    "second",
    "set",
    "short",
    "size",
    "SInt16",
    "SInt32",
    "SInt64",
    "SInt8",
    "staticvector",
    "string",
    "TextAsset",
    "TextMesh",
    "Texture",
    "Texture2D",
    "Transform",
    "TypelessData",
    "UInt16",
    "UInt32",
    "UInt64",
    "UInt8",
    "unsigned int",
    "unsigned long long",
    "unsigned short",
    "vector",
    "Vector2f",
    "Vector3f",
    "Vector4f",
    "m_ScriptingClassIdentifier",
    "Gradient",
    "Type*",
    "int2_storage",
    "int3_storage",
    "BoundsInt",
    "m_CorrespondingSourceObject",
    "m_PrefabInstance",
    "m_PrefabAsset",
    "FileSize",
    "Hash128",
];

fn common_string(offset: u32) -> Option<&'static str> {
    let mut pos = 0u32;
    for s in COMMON_STRINGS {
        if pos == offset {
            return Some(s);
        }
        pos += s.len() as u32 + 1;
    }
    None
}

fn lookup(buf: &[u8], off: u32) -> String {
    if off & 0x8000_0000 != 0 {
        let o = off & 0x7FFF_FFFF;
        return common_string(o).map(str::to_owned).unwrap_or_else(|| format!("<common:{o}>"));
    }
    let s = &buf[(off as usize).min(buf.len())..];
    let n = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..n]).into_owned()
}

pub fn read_type_tree(r: &mut Reader, version: u32) -> Result<TypeTree> {
    let count = r.len_i32()?;
    let str_size = r.len_i32()?;
    let mut raw = Vec::with_capacity(count);
    for _ in 0..count {
        let _ver = r.u16()?;
        let level = r.u8()?;
        let type_flags = r.u8()?;
        let ty = r.u32()?;
        let name = r.u32()?;
        let byte_size = r.i32()?;
        let _index = r.i32()?;
        let meta_flag = r.i32()?;
        if version >= 19 {
            let _ref_hash = r.u64()?;
        }
        raw.push((level, type_flags, ty, name, byte_size, meta_flag));
    }
    let strings = r.take(str_size)?;
    let mut nodes: Vec<TypeNode> = raw
        .into_iter()
        .map(|(level, type_flags, ty, name, byte_size, meta_flag)| TypeNode {
            ty: lookup(strings, ty),
            name: lookup(strings, name),
            byte_size,
            level,
            type_flags,
            meta_flag,
            children: Vec::new(),
        })
        .collect();
    // Link children by level (pre-order list).
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..nodes.len() {
        while let Some(&top) = stack.last() {
            if nodes[top].level < nodes[i].level {
                break;
            }
            stack.pop();
        }
        if let Some(&parent) = stack.last() {
            nodes[parent].children.push(i);
        }
        stack.push(i);
    }
    Ok(TypeTree { nodes })
}

/// Options for reading values. Byte blobs are never expanded into JSON arrays.
#[derive(Clone, Copy, Debug)]
pub struct ReadOpts {
    /// Keep at most this many elements of non-byte arrays (the rest are still parsed).
    pub max_array: usize,
    /// Keep at most this many bytes of a byte blob as hex.
    pub max_bytes: usize,
    /// Strings longer than this are summarized; non-UTF-8 strings become byte blobs.
    pub max_string: usize,
}

impl Default for ReadOpts {
    fn default() -> Self {
        Self { max_array: usize::MAX, max_bytes: usize::MAX, max_string: usize::MAX }
    }
}

#[derive(Debug)]
pub struct ValueReader<'t, 'a> {
    tree: &'t TypeTree,
    r: Reader<'a>,
    opts: ReadOpts,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Byte blobs carry their offset within the object (`$off`) so callers can slice the raw bytes.
fn blob_at(b: &[u8], off: usize, max: usize) -> Value {
    json!({ "$bytes": b.len(), "$off": off, "head": hex(&b[..b.len().min(max)]) })
}

impl<'t, 'a> ValueReader<'t, 'a> {
    pub fn new(tree: &'t TypeTree, object: &'a [u8], big_endian: bool, opts: ReadOpts) -> Self {
        Self { tree, r: Reader::new(object, big_endian), opts }
    }

    /// Read the whole object.
    pub fn read_root(&mut self) -> Result<Value> {
        self.read(0)
    }

    /// Bytes consumed so far (to check the tree matched the object size).
    pub fn consumed(&self) -> usize {
        self.r.pos()
    }

    /// Read only the leading fields of the root, stopping after `name`.
    pub fn read_until(&mut self, name: &str) -> Result<Map<String, Value>> {
        let mut out = Map::new();
        for &c in &self.tree.nodes[0].children {
            let v = self.read(c)?;
            let n = self.tree.nodes[c].name.clone();
            let stop = n == name;
            out.insert(n, v);
            if stop {
                break;
            }
        }
        Ok(out)
    }

    fn read(&mut self, i: usize) -> Result<Value> {
        let n = &self.tree.nodes[i];
        let r = &mut self.r;
        let v = match n.ty.as_str() {
            "bool" => Value::Bool(r.u8()? != 0),
            "SInt8" => json!(r.i8()?),
            "UInt8" | "char" => json!(r.u8()?),
            "SInt16" | "short" => json!(r.i16()?),
            "UInt16" | "unsigned short" => json!(r.u16()?),
            "SInt32" | "int" => json!(r.i32()?),
            "UInt32" | "unsigned int" | "Type*" => json!(r.u32()?),
            "SInt64" | "long long" => json!(r.i64()?),
            "UInt64" | "unsigned long long" | "FileSize" => json!(r.u64()?),
            "float" => json!(r.f32()?),
            "double" => json!(r.f64()?),
            "string" => {
                let len = r.len_i32()?;
                let off = r.pos();
                let raw = r.take(len)?;
                if n.children.first().is_some_and(|&c| self.tree.nodes[c].aligned()) {
                    r.align(4)?;
                }
                match std::str::from_utf8(raw) {
                    Ok(s) if s.len() <= self.opts.max_string => Value::String(s.to_owned()),
                    Ok(s) => {
                        let cut = s.char_indices().nth(self.opts.max_string).map_or(s.len(), |(i, _)| i);
                        json!({ "$str": len, "head": &s[..cut] })
                    }
                    Err(_) => blob_at(raw, off, self.opts.max_bytes),
                }
            }
            "TypelessData" => {
                let len = r.len_i32()?;
                let off = r.pos();
                blob_at(r.take(len)?, off, self.opts.max_bytes)
            }
            _ if n.is_array() => self.read_array(i)?,
            _ if n.children.len() == 1 && self.tree.nodes[n.children[0]].is_array() => {
                let c = n.children[0];
                self.read_array(c)?
            }
            _ => {
                let mut m = Map::new();
                for &c in &n.children.clone() {
                    let v = self.read(c)?;
                    m.insert(self.tree.nodes[c].name.clone(), v);
                }
                Value::Object(m)
            }
        };
        if self.tree.nodes[i].aligned() {
            self.r.align(4)?;
        }
        Ok(v)
    }

    fn read_array(&mut self, arr: usize) -> Result<Value> {
        let node = &self.tree.nodes[arr];
        if node.children.len() != 2 {
            bail!("array node {} has {} children", node.name, node.children.len());
        }
        let elem = node.children[1];
        let count = self.r.len_i32().or_else(|_| {
            // Element counts can exceed remaining bytes only for zero-size elements; treat as error.
            bail!("bad array count in {}", node.name)
        })?;
        let et = &self.tree.nodes[elem];
        let v = if matches!(et.ty.as_str(), "UInt8" | "SInt8" | "char") && et.children.is_empty() {
            let off = self.r.pos();
            blob_at(self.r.take(count)?, off, self.opts.max_bytes)
        } else {
            let keep = count.min(self.opts.max_array);
            let mut out = Vec::with_capacity(keep);
            for k in 0..count {
                let v = self.read(elem)?;
                if k < keep {
                    out.push(v);
                }
            }
            if count > keep { json!({ "$len": count, "items": out }) } else { Value::Array(out) }
        };
        if self.tree.nodes[arr].aligned() {
            self.r.align(4)?;
        }
        Ok(v)
    }
}
