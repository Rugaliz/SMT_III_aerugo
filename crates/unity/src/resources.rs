//! The player's resource index: which objects `Resources` paths name.

use anyhow::{Result, bail};

use crate::reader::Reader;
use crate::serialized::SerializedFile;

/// Unity class id of the resource manager.
pub const RESOURCE_MANAGER_CLASS: i32 = 147;

/// One `Resources` path: the lower-case path and the object it names (file id as in the manager's file, path id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceEntry {
    pub path: String,
    pub file_id: i32,
    pub path_id: i64,
}

/// The container of the resource manager object of `sf` (stored without a type tree in the player files).
/// Layout: entry count, then per entry an aligned length-prefixed string and an object reference (file id,
/// path id).
pub fn resource_container(sf: &SerializedFile<'_>) -> Result<Vec<ResourceEntry>> {
    let Some(o) = sf.objects.iter().find(|o| sf.types[o.type_index].class_id == RESOURCE_MANAGER_CLASS)
    else {
        bail!("no resource manager");
    };
    let mut r = Reader::new(sf.object_bytes(o)?, sf.big_endian);
    let n = r.len_i32()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let len = r.len_i32()?;
        let path = String::from_utf8_lossy(r.take(len)?).into_owned();
        r.align(4)?;
        let file_id = r.i32()?;
        let path_id = r.i64()?;
        out.push(ResourceEntry { path, file_id, path_id });
    }
    Ok(out)
}
