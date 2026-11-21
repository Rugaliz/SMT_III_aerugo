//! Bounds-checked byte cursor with switchable endianness.

use anyhow::{Result, bail};

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    pub big_endian: bool,
}

macro_rules! prim {
    ($name:ident, $t:ty) => {
        pub fn $name(&mut self) -> Result<$t> {
            let b = self.take(size_of::<$t>())?;
            let arr = b.try_into().unwrap();
            Ok(if self.big_endian { <$t>::from_be_bytes(arr) } else { <$t>::from_le_bytes(arr) })
        }
    };
}

impl std::fmt::Debug for Reader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader").field("len", &self.data.len()).field("pos", &self.pos).finish()
    }
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], big_endian: bool) -> Self {
        Self { data, pos: 0, big_endian }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            bail!("seek to {pos} past end {}", self.data.len());
        }
        self.pos = pos;
        Ok(())
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let Some(end) = end else {
            bail!("read of {n} bytes at {} past end {}", self.pos, self.data.len());
        };
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    /// Align to `n` bytes relative to the start of the buffer.
    pub fn align(&mut self, n: usize) -> Result<()> {
        let p = self.pos.div_ceil(n) * n;
        self.seek(p.min(self.data.len()))
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    prim!(u16, u16);
    prim!(i16, i16);
    prim!(u32, u32);
    prim!(i32, i32);
    prim!(u64, u64);
    prim!(i64, i64);
    prim!(f32, f32);
    prim!(f64, f64);

    /// NUL-terminated string.
    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let Some(n) = rest.iter().position(|&b| b == 0) else {
            bail!("unterminated string at {}", self.pos);
        };
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.pos += n + 1;
        Ok(s)
    }

    /// Non-negative i32 length, sanity-checked against the remaining buffer.
    pub fn len_i32(&mut self) -> Result<usize> {
        let n = self.i32()?;
        if n < 0 || n as usize > self.data.len() - self.pos {
            bail!("bad length {n} at {}", self.pos - 4);
        }
        Ok(n as usize)
    }
}
