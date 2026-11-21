//! Windows Media Audio version 2, as used by the game's movies.
//!
//! Each ASF audio object is a *superframe* holding a few 2048-sample frames; frames may continue into the next
//! superframe (a bit reservoir). A frame is split into transform blocks of 2048 down to 128 samples, each
//! carrying spectral coefficients coded as runs and levels, scaled by per-band exponents and recovered with an
//! inverse MDCT and sine windows.
//!
//! Only the configuration of the game's movies is supported (two channels, 48 kHz, 128 kbit/s); others are
//! rejected.

mod mdct;
mod tables;

use anyhow::{Context, Result, ensure};

use crate::asf::AudioFormat;
use crate::bits::{BitReader, Vlc};
use mdct::Imdct;

const FORMAT_WMA2: u16 = 0x161;
const FRAME_LEN_BITS: u32 = 11;
const FRAME_LEN: usize = 1 << FRAME_LEN_BITS;
const BLOCK_MIN_BITS: u32 = 7;
const MAX_CARRY: usize = 32768;

/// Exponent value for a running exponent index.
fn pow_exp(index: i32) -> f32 {
    10f64.powf(index as f64 / 16.0) as f32
}

struct CoefTable {
    vlc: Vlc,
    run: Vec<u16>,
    level: Vec<f32>,
}

impl CoefTable {
    fn new(codes: &[u32], bits: &[u8], levels: &[u16]) -> Result<Self> {
        let n = codes.len();
        let vlc = Vlc::new(codes.iter().copied().zip(bits.iter().copied()))?;
        let mut run = vec![0u16; n];
        let mut level = vec![0f32; n];
        let mut i = 2;
        for (l, &count) in levels.iter().enumerate() {
            for j in 0..count {
                if i >= n {
                    break;
                }
                run[i] = j;
                level[i] = (l + 1) as f32;
                i += 1;
            }
        }
        Ok(Self { vlc, run, level })
    }
}

/// Decoder state.
pub struct WmaDecoder {
    channels: usize,
    nb_block_sizes: usize,
    byte_offset_bits: u32,
    exponent_bands: Vec<Vec<u16>>,
    coefs_end: Vec<usize>,
    scale_vlc: Vlc,
    coef: [CoefTable; 2],
    imdct: Vec<Imdct>,
    windows: Vec<Vec<f32>>,
    // block state
    reset_block_lengths: bool,
    prev_block_len_bits: u32,
    block_len_bits: u32,
    next_block_len_bits: u32,
    block_pos: usize,
    exponents: [Vec<f32>; 2],
    exponents_bsize: [usize; 2],
    exponents_initialized: [bool; 2],
    max_exponent: [f32; 2],
    frame_out: [Vec<f32>; 2],
    // bit reservoir
    carry: Vec<u8>,
    last_bitoffset: usize,
    /// Superframes that only extended the carried-over frame.
    pub continuation_only: usize,
}

impl std::fmt::Debug for WmaDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WmaDecoder")
            .field("channels", &self.channels)
            .field("blocks", &self.nb_block_sizes)
            .finish()
    }
}

impl WmaDecoder {
    pub fn new(fmt: &AudioFormat) -> Result<Self> {
        ensure!(fmt.format_tag == FORMAT_WMA2, "audio format {:#x} is not WMA v2", fmt.format_tag);
        ensure!(fmt.channels == 2 && fmt.rate == 48000, "only 2-channel 48 kHz WMA is supported");
        ensure!(fmt.extra.len() >= 6, "WMA extra data too short");
        let flags2 = u16::from_le_bytes([fmt.extra[4], fmt.extra[5]]);
        ensure!(
            flags2 & 0x7 == 0x7,
            "WMA flags {flags2:#x} not supported (need exponent codes, reservoir, variable blocks)"
        );
        let channels = fmt.channels as usize;
        let bit_rate = fmt.bytes_per_second as f32 * 8.0;
        let rate = fmt.rate as f32;
        // Number of block sizes.
        let mut nb = ((flags2 >> 3) & 3) as u32 + 1;
        if fmt.bytes_per_second * 8 / channels as u32 >= 32000 {
            nb += 2;
        }
        nb = nb.min(FRAME_LEN_BITS - BLOCK_MIN_BITS);
        let nb_block_sizes = nb as usize + 1;
        let bps = bit_rate / (channels as f32 * rate);
        let bps1 = if channels == 2 { bps * 1.6 } else { bps };
        ensure!(bps1 >= 0.61, "WMA with noise coding is not supported");
        ensure!(bps1 >= 1.16, "only the highest-rate WMA code tables are supported");
        let byte_offset_bits = ((bps * FRAME_LEN as f32 / 8.0 + 0.5) as i32).ilog2() + 2;

        let mut exponent_bands = Vec::new();
        let mut coefs_end = Vec::new();
        for k in 0..nb_block_sizes {
            let block_len = FRAME_LEN >> k;
            let a = FRAME_LEN_BITS as i32 - BLOCK_MIN_BITS as i32 - k as i32;
            let bands: Vec<u16> = if a < 3 {
                let t = tables::EXPONENT_BANDS_44100[a as usize];
                t[1..=t[0] as usize].iter().map(|&v| v as u16).collect()
            } else {
                let mut v = Vec::new();
                let mut lpos = 0usize;
                for &f in &tables::CRITICAL_FREQS {
                    let b = fmt.rate as usize;
                    let mut pos = (block_len * 2 * f as usize + (b << 1)) / (4 * b);
                    pos <<= 2;
                    pos = pos.min(block_len);
                    if pos > lpos {
                        v.push((pos - lpos) as u16);
                    }
                    if pos >= block_len {
                        break;
                    }
                    lpos = pos;
                }
                v
            };
            exponent_bands.push(bands);
            coefs_end.push((FRAME_LEN - FRAME_LEN * 9 / 100) >> k);
        }
        let scale_vlc =
            Vlc::new(tables::SCALE_CODES.iter().copied().zip(tables::SCALE_BITS.iter().copied()))?;
        let coef = [
            CoefTable::new(&tables::COEF4_CODES, &tables::COEF4_BITS, &tables::LEVELS4)?,
            CoefTable::new(&tables::COEF5_CODES, &tables::COEF5_BITS, &tables::LEVELS5)?,
        ];
        let mut imdct = Vec::new();
        let mut windows = Vec::new();
        for k in 0..nb_block_sizes {
            let n = FRAME_LEN >> k;
            imdct.push(Imdct::new(n, 1.0 / 32768.0));
            windows.push(
                (0..n)
                    .map(|i| ((i as f64 + 0.5) * (std::f64::consts::PI / (2.0 * n as f64))).sin() as f32)
                    .collect(),
            );
        }
        Ok(Self {
            channels,
            nb_block_sizes,
            byte_offset_bits,
            exponent_bands,
            coefs_end,
            scale_vlc,
            coef,
            imdct,
            windows,
            reset_block_lengths: true,
            prev_block_len_bits: FRAME_LEN_BITS,
            block_len_bits: FRAME_LEN_BITS,
            next_block_len_bits: FRAME_LEN_BITS,
            block_pos: 0,
            exponents: [vec![0.0; FRAME_LEN], vec![0.0; FRAME_LEN]],
            exponents_bsize: [0; 2],
            exponents_initialized: [false; 2],
            max_exponent: [1.0; 2],
            frame_out: [vec![0.0; FRAME_LEN * 2], vec![0.0; FRAME_LEN * 2]],
            carry: Vec::new(),
            last_bitoffset: 0,
            continuation_only: 0,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Sample frames at the start of the output that are decoder warm-up, not part of the stream (two frames).
    pub fn delay(&self) -> usize {
        2 * FRAME_LEN
    }

    /// Decode one superframe; returns interleaved samples (possibly none).
    pub fn decode(&mut self, sf: &[u8]) -> Result<Vec<f32>> {
        let r = self.decode_inner(sf);
        if r.is_err() {
            self.carry.clear();
        }
        r
    }

    fn decode_inner(&mut self, sf: &[u8]) -> Result<Vec<f32>> {
        let mut out = Vec::new();
        let mut g = BitReader::new(sf);
        g.skip(4)?;
        let mut nb_frames = g.bits(4)? as i32 - i32::from(self.carry.is_empty());
        if nb_frames <= 0 {
            ensure!(nb_frames == 0 && g.left() > 8, "bad WMA superframe frame count");
            ensure!(self.carry.len() + sf.len() - 1 <= MAX_CARRY, "WMA carry buffer overflow");
            for _ in 0..sf.len() - 1 {
                self.carry.push(g.bits(8)? as u8);
            }
            self.continuation_only += 1;
            return Ok(out);
        }
        let bit_offset = g.bits(self.byte_offset_bits + 3)? as usize;
        ensure!(bit_offset as isize <= g.left(), "WMA bit offset past the superframe");
        if !self.carry.is_empty() {
            let carried = self.carry.len();
            let mut len = bit_offset;
            while len > 7 {
                self.carry.push(g.bits(8)? as u8);
                len -= 8;
            }
            if len > 0 {
                self.carry.push((g.bits(len as u32)? << (8 - len)) as u8);
            }
            ensure!(self.carry.len() <= MAX_CARRY, "WMA carry buffer overflow");
            let carry = std::mem::take(&mut self.carry);
            let mut cg = BitReader::with_len(&carry, carried * 8 + bit_offset);
            cg.skip(self.last_bitoffset)?;
            self.decode_frame(&mut cg, &mut out)?;
            nb_frames -= 1;
        }
        let start = bit_offset + 4 + 4 + self.byte_offset_bits as usize + 3;
        ensure!(start <= sf.len() * 8, "WMA frame start past the superframe");
        let tail = &sf[start >> 3..];
        let mut fg = BitReader::new(tail);
        fg.skip(start & 7)?;
        self.reset_block_lengths = true;
        for _ in 0..nb_frames {
            self.decode_frame(&mut fg, &mut out)?;
        }
        let pos = fg.pos() + (start & !7);
        self.last_bitoffset = pos & 7;
        let byte = pos >> 3;
        ensure!(byte <= sf.len(), "WMA frame data past the superframe");
        self.carry = sf[byte..].to_vec();
        Ok(out)
    }

    fn decode_frame(&mut self, g: &mut BitReader<'_>, out: &mut Vec<f32>) -> Result<()> {
        self.block_pos = 0;
        while !self.decode_block(g)? {}
        let base = out.len();
        out.resize(base + FRAME_LEN * self.channels, 0.0);
        for ch in 0..self.channels {
            for i in 0..FRAME_LEN {
                out[base + i * self.channels + ch] = self.frame_out[ch][i];
            }
            self.frame_out[ch].copy_within(FRAME_LEN.., 0);
            self.frame_out[ch][FRAME_LEN..].fill(0.0);
        }
        Ok(())
    }

    /// Returns true when the frame is complete.
    // Channels index several parallel per-channel arrays.
    #[allow(clippy::needless_range_loop)]
    fn decode_block(&mut self, g: &mut BitReader<'_>) -> Result<bool> {
        let n = (self.nb_block_sizes as u32 - 1).ilog2() + 1;
        let size_bits = |g: &mut BitReader<'_>, nb: usize| -> Result<u32> {
            let v = g.bits(n)?;
            ensure!((v as usize) < nb, "WMA block size {v} out of range");
            Ok(FRAME_LEN_BITS - v)
        };
        if self.reset_block_lengths {
            self.reset_block_lengths = false;
            self.prev_block_len_bits = size_bits(g, self.nb_block_sizes)?;
            self.block_len_bits = size_bits(g, self.nb_block_sizes)?;
        } else {
            self.prev_block_len_bits = self.block_len_bits;
            self.block_len_bits = self.next_block_len_bits;
        }
        self.next_block_len_bits = size_bits(g, self.nb_block_sizes)?;
        let block_len = 1usize << self.block_len_bits;
        ensure!(self.block_pos + block_len <= FRAME_LEN, "WMA block past the frame");
        let bsize = (FRAME_LEN_BITS - self.block_len_bits) as usize;

        let ms_stereo = self.channels == 2 && g.bit()?;
        let mut coded = [false; 2];
        for c in coded.iter_mut().take(self.channels) {
            *c = g.bit()?;
        }
        let mut coefs = [vec![0f32; block_len], vec![0f32; block_len]];
        if coded.iter().any(|&c| c) {
            let mut total_gain = 1i32;
            loop {
                ensure!(g.left() >= 7, "WMA gain past the data");
                let a = g.bits(7)? as i32;
                total_gain += a;
                if a != 127 {
                    break;
                }
            }
            let coef_nb_bits = match total_gain {
                ..15 => 13,
                15..32 => 12,
                32..40 => 11,
                40..45 => 10,
                _ => 9,
            };
            let ncoefs = self.coefs_end[bsize];
            if self.block_len_bits == FRAME_LEN_BITS || g.bit()? {
                for ch in 0..self.channels {
                    if coded[ch] {
                        self.decode_exponents(g, ch, bsize, block_len)?;
                        self.exponents_bsize[ch] = bsize;
                        self.exponents_initialized[ch] = true;
                    }
                }
            }
            for ch in 0..self.channels {
                ensure!(!coded[ch] || self.exponents_initialized[ch], "WMA channel without exponents");
            }
            let mut raw = [vec![0f32; block_len], vec![0f32; block_len]];
            for ch in 0..self.channels {
                if coded[ch] {
                    let t = usize::from(ch == 1 && ms_stereo);
                    self.run_level(g, t, &mut raw[ch], ncoefs, block_len, coef_nb_bits)?;
                }
            }
            let mdct_norm = 1.0 / (block_len / 2) as f32;
            for ch in 0..self.channels {
                if coded[ch] {
                    let esize = self.exponents_bsize[ch];
                    let mult =
                        10f64.powf(total_gain as f64 * 0.05) as f32 / self.max_exponent[ch] * mdct_norm;
                    let e = &self.exponents[ch];
                    for i in 0..ncoefs {
                        coefs[ch][i] = raw[ch][i] * e[(i << bsize) >> esize] * mult;
                    }
                }
            }
            if ms_stereo && coded[1] {
                if !coded[0] {
                    coefs[0].fill(0.0);
                    coded[0] = true;
                }
                let (a, b) = coefs.split_at_mut(1);
                for (x, y) in a[0].iter_mut().zip(b[0].iter_mut()) {
                    let (s, d) = (*x + *y, *x - *y);
                    *x = s;
                    *y = d;
                }
            }
        }
        let mut output = vec![0f32; block_len * 2];
        for ch in 0..self.channels {
            if coded[ch] {
                self.imdct[bsize].full(&coefs[ch], &mut output);
            } else if !(ms_stereo && ch == 1) {
                output.fill(0.0);
            }
            let index = FRAME_LEN / 2 + self.block_pos - block_len / 2;
            self.window(ch, index, &output, block_len);
        }
        self.block_pos += block_len;
        Ok(self.block_pos >= FRAME_LEN)
    }

    fn decode_exponents(
        &mut self,
        g: &mut BitReader<'_>,
        ch: usize,
        bsize: usize,
        block_len: usize,
    ) -> Result<()> {
        let mut last = 36i32;
        let mut max = 0f32;
        let mut q = 0usize;
        let mut band = 0usize;
        while q < block_len {
            let code = self.scale_vlc.read(g)? as i32;
            last += code - 60;
            ensure!((0..156).contains(&(last + 60)), "WMA exponent out of range");
            let v = pow_exp(last);
            max = max.max(v);
            let n = *self.exponent_bands[bsize].get(band).context("WMA exponent bands exhausted")? as usize;
            band += 1;
            for _ in 0..n {
                if q < block_len {
                    self.exponents[ch][q] = v;
                }
                q += 1;
            }
        }
        self.max_exponent[ch] = max;
        Ok(())
    }

    fn run_level(
        &self,
        g: &mut BitReader<'_>,
        t: usize,
        out: &mut [f32],
        num: usize,
        block_len: usize,
        coef_nb_bits: u32,
    ) -> Result<()> {
        let tab = &self.coef[t];
        let mask = block_len - 1;
        let mut offset = 0usize;
        while offset < num {
            let code = tab.vlc.read(g)? as usize;
            if code > 1 {
                offset += tab.run[code] as usize;
                let neg = !g.bit()?;
                let l = tab.level[code];
                out[offset & mask] = if neg { -l } else { l };
            } else if code == 1 {
                break;
            } else {
                // Escape: explicit level and run.
                let level = g.bits(coef_nb_bits)? as i32;
                offset += g.bits(FRAME_LEN_BITS)? as usize;
                let neg = !g.bit()?;
                out[offset & mask] = if neg { -level as f32 } else { level as f32 };
            }
            offset += 1;
        }
        ensure!(offset <= num, "WMA coefficient run past the block");
        Ok(())
    }

    fn window(&mut self, ch: usize, index: usize, input: &[f32], block_len: usize) {
        let out = &mut self.frame_out[ch][index..];
        // Left half.
        if self.block_len_bits <= self.prev_block_len_bits {
            let w = &self.windows[(FRAME_LEN_BITS - self.block_len_bits) as usize];
            for i in 0..block_len {
                out[i] += input[i] * w[i];
            }
        } else {
            let bl = 1usize << self.prev_block_len_bits;
            let n = (block_len - bl) / 2;
            let w = &self.windows[(FRAME_LEN_BITS - self.prev_block_len_bits) as usize];
            for i in 0..bl {
                out[n + i] += input[n + i] * w[i];
            }
            out[n + bl..n + bl + n].copy_from_slice(&input[n + bl..n + bl + n]);
        }
        // Right half.
        let out = &mut out[block_len..];
        let input = &input[block_len..];
        if self.block_len_bits <= self.next_block_len_bits {
            let w = &self.windows[(FRAME_LEN_BITS - self.block_len_bits) as usize];
            for i in 0..block_len {
                out[i] = input[i] * w[block_len - 1 - i];
            }
        } else {
            let bl = 1usize << self.next_block_len_bits;
            let n = (block_len - bl) / 2;
            let w = &self.windows[(FRAME_LEN_BITS - self.next_block_len_bits) as usize];
            out[..n].copy_from_slice(&input[..n]);
            for i in 0..bl {
                out[n + i] = input[n + i] * w[bl - 1 - i];
            }
            out[n + bl..n + bl + n].fill(0.0);
        }
    }
}
