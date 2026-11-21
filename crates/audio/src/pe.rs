//! Just enough of the Windows PE format to map virtual addresses to file offsets.

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub(crate) struct Section {
    pub name: String,
    pub va: u64,
    pub vsize: u64,
    pub raw: u64,
    pub raw_size: u64,
}

#[derive(Debug)]
pub(crate) struct Pe {
    pub image_base: u64,
    pub sections: Vec<Section>,
}

fn u16_at(d: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(d.get(o..o + 2).context("truncated PE")?.try_into()?))
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(d.get(o..o + 4).context("truncated PE")?.try_into()?))
}

impl Pe {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.get(..2) != Some(b"MZ") {
            bail!("not a PE file");
        }
        let pe = u32_at(data, 0x3C)? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            bail!("missing PE signature");
        }
        let count = u16_at(data, pe + 6)? as usize;
        let opt_size = u16_at(data, pe + 20)? as usize;
        let opt = pe + 24;
        if u16_at(data, opt)? != 0x20B {
            bail!("not a 64-bit image");
        }
        let image_base =
            u64::from_le_bytes(data.get(opt + 24..opt + 32).context("truncated PE")?.try_into()?);
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let s = opt + opt_size + i * 40;
            let name = data.get(s..s + 8).context("truncated section table")?;
            let name = String::from_utf8_lossy(name).trim_end_matches('\0').to_string();
            sections.push(Section {
                name,
                vsize: u32_at(data, s + 8)?.into(),
                va: u32_at(data, s + 12)?.into(),
                raw_size: u32_at(data, s + 16)?.into(),
                raw: u32_at(data, s + 20)?.into(),
            });
        }
        Ok(Self { image_base, sections })
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// File offset of virtual address `va`, if it is stored in the file.
    pub fn offset_of(&self, va: u64) -> Option<usize> {
        let rel = va.checked_sub(self.image_base)?;
        self.sections
            .iter()
            .find(|s| rel >= s.va && rel < s.va + s.raw_size.min(s.vsize))
            .map(|s| (s.raw + rel - s.va) as usize)
    }
}
