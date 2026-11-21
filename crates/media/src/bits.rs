//! Bit-level reading (most significant bit first) and variable-length codes.

use anyhow::{Result, bail};

/// Reads bits from a byte slice, most significant bit first.
#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Position in bits.
    pos: usize,
    /// Length in bits.
    len: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, len: data.len() * 8 }
    }

    /// A reader over the first `bits` bits of `data`.
    pub fn with_len(data: &'a [u8], bits: usize) -> Self {
        Self { data, pos: 0, len: bits.min(data.len() * 8) }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn left(&self) -> isize {
        self.len as isize - self.pos as isize
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        if self.pos + n > self.len {
            bail!("read past the end of the bitstream");
        }
        self.pos += n;
        Ok(())
    }

    /// Up to 32 bits.
    pub fn bits(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        if self.pos + n as usize > self.len {
            bail!("read past the end of the bitstream");
        }
        let mut v: u64 = 0;
        let mut got = 0;
        while got < n {
            let byte = self.data[self.pos / 8];
            let bit_off = (self.pos % 8) as u32;
            let take = (8 - bit_off).min(n - got);
            let chunk = (byte >> (8 - bit_off - take)) & ((1u16 << take) - 1) as u8;
            v = (v << take) | chunk as u64;
            got += take;
            self.pos += take as usize;
        }
        Ok(v as u32)
    }

    pub fn bit(&mut self) -> Result<bool> {
        Ok(self.bits(1)? != 0)
    }

    #[allow(dead_code)] // for the video decoder
    /// Peek up to 32 bits without moving; missing bits past the end read as zero.
    pub fn peek(&self, n: u32) -> u32 {
        let mut c = self.clone();
        c.len = c.data.len() * 8;
        let avail = (c.len - c.pos.min(c.len)).min(n as usize) as u32;
        let v = c.bits(avail).unwrap_or(0);
        if avail < n { v << (n - avail) } else { v }
    }

    #[allow(dead_code)] // for the video decoder
    pub fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// A prefix code table: decodes to symbol indices.
#[derive(Clone, Debug)]
pub struct Vlc {
    /// Binary tree: `nodes[i] = [child for 0, child for 1]`; a child is `NODE | index` or `LEAF | symbol`.
    nodes: Vec<[u32; 2]>,
}

const LEAF: u32 = 1 << 31;
const EMPTY: u32 = u32::MAX;

impl Vlc {
    /// Build from `(code, length)` per symbol; symbols with length 0 are absent.
    pub fn new(codes: impl IntoIterator<Item = (u32, u8)>) -> Result<Self> {
        let mut nodes = vec![[EMPTY; 2]];
        for (sym, (code, len)) in codes.into_iter().enumerate() {
            if len == 0 {
                continue;
            }
            let mut n = 0usize;
            for i in (0..len).rev() {
                let b = ((code >> i) & 1) as usize;
                if i == 0 {
                    if nodes[n][b] != EMPTY {
                        bail!("prefix code is not prefix-free (symbol {sym})");
                    }
                    nodes[n][b] = LEAF | sym as u32;
                } else {
                    match nodes[n][b] {
                        EMPTY => {
                            nodes.push([EMPTY; 2]);
                            let id = (nodes.len() - 1) as u32;
                            nodes[n][b] = id;
                            n = id as usize;
                        }
                        c if c & LEAF != 0 => bail!("prefix code is not prefix-free (symbol {sym})"),
                        c => n = c as usize,
                    }
                }
            }
        }
        Ok(Self { nodes })
    }

    pub fn read(&self, r: &mut BitReader<'_>) -> Result<u32> {
        let mut n = 0usize;
        loop {
            let c = self.nodes[n][r.bit()? as usize];
            if c == EMPTY {
                bail!("invalid code");
            }
            if c & LEAF != 0 {
                return Ok(c & !LEAF);
            }
            n = c as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_codes() {
        let d = [0b1011_0011, 0b0101_0000];
        let mut r = BitReader::new(&d);
        assert_eq!(r.bits(3).unwrap(), 0b101);
        assert_eq!(r.bits(7).unwrap(), 0b100_1101);
        assert_eq!(r.peek(4), 0b0100);
        r.align();
        assert_eq!(r.pos(), 16);
        // Codes: 0 -> "0", 1 -> "10", 2 -> "11".
        let v = Vlc::new([(0, 1), (0b10, 2), (0b11, 2)]).unwrap();
        let d = [0b0101_1000];
        let mut r = BitReader::new(&d);
        assert_eq!([v.read(&mut r).unwrap(), v.read(&mut r).unwrap(), v.read(&mut r).unwrap()], [0, 1, 2]);
    }
}
