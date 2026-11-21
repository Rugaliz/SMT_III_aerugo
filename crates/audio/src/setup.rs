//! Vorbis setup headers from the player library of the installation.
//!
//! The library holds a table of 32-byte entries:
//! `{ u64 address, i32 size, u32 crc32, u64 base address, i32 offset, i32 length }`.
//! With a zero base address, `address`/`size` is the whole header. Otherwise the header is the `size` bytes at
//! `base address`, with `length` bytes from `address` written over it at `offset`. A header is accepted only
//! if it starts with the setup packet magic and its CRC-32 matches the entry.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::crc32::crc32;
use crate::pe::Pe;

const MAGIC: &[u8; 7] = b"\x05vorbis";
const ENTRY: usize = 32;
const MAX_HEADER: usize = 0x10000;

/// Setup headers by CRC-32.
#[derive(Debug, Default)]
pub struct SetupHeaders {
    headers: HashMap<u32, Vec<u8>>,
}

impl SetupHeaders {
    /// Read the headers from the player library (`UnityPlayer.dll` in the installation directory).
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Self::from_library(&data)
    }

    pub fn from_library(data: &[u8]) -> Result<Self> {
        let pe = Pe::parse(data)?;
        let rdata = pe.section(".rdata").context("no .rdata section")?;
        let (start, end) = (rdata.raw as usize, (rdata.raw + rdata.raw_size) as usize);
        let bytes = |va: u64, len: usize| -> Option<&[u8]> {
            let o = pe.offset_of(va)?;
            data.get(o..o.checked_add(len)?)
        };
        let mut headers = HashMap::new();
        let mut o = start.next_multiple_of(8);
        while o + ENTRY <= end {
            let e = &data[o..o + ENTRY];
            let word = |k: usize| u64::from_le_bytes(e[k..k + 8].try_into().unwrap());
            let int = |k: usize| i32::from_le_bytes(e[k..k + 4].try_into().unwrap());
            let (addr, size, crc, base) = (word(0), int(8), int(12) as u32, word(16));
            let (off, len) = (int(24), int(28));
            o += 8;
            if size <= MAGIC.len() as i32 || size as usize > MAX_HEADER || headers.contains_key(&crc) {
                continue;
            }
            let size = size as usize;
            let header = if base == 0 {
                if off != 0 || len != 0 {
                    continue;
                }
                match bytes(addr, size) {
                    Some(b) => b.to_vec(),
                    None => continue,
                }
            } else {
                let (Ok(off), Ok(len)) = (usize::try_from(off), usize::try_from(len)) else { continue };
                if len == 0 || off + len > size {
                    continue;
                }
                let (Some(b), Some(patch)) = (bytes(base, size), bytes(addr, len)) else { continue };
                let mut h = b.to_vec();
                h[off..off + len].copy_from_slice(patch);
                h
            };
            if header.starts_with(MAGIC) && crc32(&header) == crc {
                headers.insert(crc, header);
            }
        }
        if headers.is_empty() {
            bail!("no Vorbis setup headers found in the player library");
        }
        Ok(Self { headers })
    }

    pub fn get(&self, crc: u32) -> Option<&[u8]> {
        self.headers.get(&crc).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.headers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }
}
