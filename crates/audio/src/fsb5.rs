//! FSB5 sound banks (version 1, Vorbis).

use anyhow::{Context, Result, bail};

/// Codec field value for Vorbis.
const MODE_VORBIS: u32 = 15;
/// Size of the version 1 bank header.
const HEADER_SIZE: usize = 60;

/// Chunk types in a sample header.
const CHUNK_LOOP: u32 = 3;
const CHUNK_VORBIS: u32 = 11;

/// A loop region in sample frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loop {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub rate: u32,
    pub channels: u8,
    /// Length in sample frames.
    pub frames: u64,
    /// CRC-32 of the Vorbis setup header the sample was encoded with.
    pub setup_crc: u32,
    pub looping: Option<Loop>,
    /// Byte range of the sample's packets in [`Bank::data`].
    pub data: std::ops::Range<usize>,
}

/// A parsed bank borrowing its bytes.
#[derive(Debug)]
pub struct Bank<'a> {
    pub samples: Vec<Sample>,
    pub data: &'a [u8],
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(d.get(o..o + 4).context("truncated bank")?.try_into()?))
}

impl<'a> Bank<'a> {
    pub fn parse(d: &'a [u8]) -> Result<Self> {
        if d.get(..4) != Some(b"FSB5") {
            bail!("not an FSB5 bank");
        }
        let version = u32_at(d, 4)?;
        if version != 1 {
            bail!("FSB5 version {version} is not supported");
        }
        let count = u32_at(d, 8)? as usize;
        let headers_size = u32_at(d, 12)? as usize;
        let names_size = u32_at(d, 16)? as usize;
        let data_size = u32_at(d, 20)? as usize;
        let mode = u32_at(d, 24)?;
        if mode != MODE_VORBIS {
            bail!("FSB5 codec {mode} is not supported");
        }
        let data_start = HEADER_SIZE + headers_size + names_size;
        let data = d.get(data_start..data_start + data_size).context("bank data out of range")?;

        let mut pos = HEADER_SIZE;
        let mut samples = Vec::with_capacity(count);
        let mut starts = Vec::with_capacity(count);
        for _ in 0..count {
            let v = u64::from_le_bytes(d.get(pos..pos + 8).context("truncated sample header")?.try_into()?);
            pos += 8;
            let rate = match (v >> 1) & 0xF {
                8 => 44100,
                9 => 48000,
                i => bail!("sample rate index {i} is not supported"),
            };
            let channels = if (v >> 5) & 1 != 0 { 2 } else { 1 };
            starts.push(((v >> 6) & 0x0FFF_FFFF) as usize * 32);
            let mut s = Sample { rate, channels, frames: v >> 34, setup_crc: 0, looping: None, data: 0..0 };
            let mut crc = None;
            let mut more = v & 1 != 0;
            while more {
                let c = u32_at(d, pos)?;
                more = c & 1 != 0;
                let size = ((c >> 1) & 0xFF_FFFF) as usize;
                let kind = c >> 25;
                let body = d.get(pos + 4..pos + 4 + size).context("truncated chunk")?;
                pos += 4 + size;
                match kind {
                    CHUNK_VORBIS => crc = Some(u32_at(body, 0)?),
                    CHUNK_LOOP => s.looping = Some(Loop { start: u32_at(body, 0)?, end: u32_at(body, 4)? }),
                    1 | 2 => bail!("channel/rate override chunks are not supported"),
                    _ => {}
                }
            }
            s.setup_crc = crc.context("sample without Vorbis data chunk")?;
            samples.push(s);
        }
        for i in 0..samples.len() {
            let end = starts.get(i + 1).copied().unwrap_or(data.len());
            if starts[i] > end || end > data.len() {
                bail!("sample {i} data out of range");
            }
            samples[i].data = starts[i]..end;
        }
        Ok(Self { samples, data })
    }

    /// The Vorbis packets of a sample: each is stored as a 16-bit little-endian length and the packet bytes.
    pub fn packets(&self, sample: &Sample) -> Result<Vec<&'a [u8]>> {
        let d = &self.data[sample.data.clone()];
        let mut out = Vec::new();
        let mut p = 0;
        while p < d.len() {
            let len = match d.get(p..p + 2) {
                Some(b) => u16::from_le_bytes(b.try_into()?) as usize,
                None => 0,
            };
            if len == 0 {
                // End of the packets: only zero padding may follow.
                if d[p..].iter().any(|&b| b != 0) {
                    bail!("data after the end of the packets");
                }
                break;
            }
            out.push(d.get(p + 2..p + 2 + len).context("truncated packet")?);
            p += 2 + len;
        }
        Ok(out)
    }
}
