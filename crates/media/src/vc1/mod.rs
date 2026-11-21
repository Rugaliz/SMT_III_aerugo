//! WMV3 video decoder: the Main profile of the VC-1 format, in the configuration the game's movies use
//! (progressive I and P frames, no B frames, fixed picture quantizer per frame). Decoded pictures match the
//! format's reference decoding sample for sample.

mod blocks;
mod dsp;
mod filter;
mod mc;
mod tables;

use anyhow::{Context as _, Result, bail, ensure};

use crate::asf::VideoFormat;
use crate::bits::{BitReader, Vlc};
use tables::*;

/// One plane of 8-bit samples, rows top to bottom.
#[derive(Clone, Debug)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Plane {
    fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0; width * height] }
    }
}

/// A decoded 4:2:0 picture. The planes cover whole macroblocks; the visible part is `display_width` x
/// `display_height` from the top left.
#[derive(Clone, Debug)]
pub struct Picture {
    pub y: Plane,
    pub cb: Plane,
    pub cr: Plane,
    pub display_width: usize,
    pub display_height: usize,
}

impl Picture {
    fn new(mbw: usize, mbh: usize, display_width: usize, display_height: usize) -> Self {
        Self {
            y: Plane::new(mbw * 16, mbh * 16),
            cb: Plane::new(mbw * 8, mbh * 8),
            cr: Plane::new(mbw * 8, mbh * 8),
            display_width,
            display_height,
        }
    }

    fn plane(&self, i: usize) -> &Plane {
        match i {
            0 => &self.y,
            1 => &self.cb,
            _ => &self.cr,
        }
    }

    fn plane_mut(&mut self, i: usize) -> &mut Plane {
        match i {
            0 => &mut self.y,
            1 => &mut self.cb,
            _ => &mut self.cr,
        }
    }
}

/// Stream-level coding tools (from the codec data).
#[derive(Clone, Copy, Debug)]
struct Sequence {
    loop_filter: bool,
    extended_mv: bool,
    vstransform: bool,
}

/// Parse the 4-byte sequence header; anything outside the supported configuration is an error.
fn parse_sequence(extra: &[u8]) -> Result<Sequence> {
    ensure!(extra.len() >= 4, "WMV3 codec data is {} bytes", extra.len());
    let mut r = BitReader::new(&extra[..4]);
    let profile = r.bits(2)?;
    ensure!(profile == 1, "WMV3 profile {profile} is not supported (Main only)");
    ensure!(!r.bit()?, "4:1:1 sampling is not supported");
    ensure!(!r.bit()?, "sprite streams are not supported");
    r.skip(8)?; // post-processing hints: frame rate and bit rate classes
    let loop_filter = r.bit()?;
    ensure!(!r.bit()?, "the alternative intra coder is not supported");
    ensure!(!r.bit()?, "multi-resolution coding is not supported");
    ensure!(r.bit()?, "only the fast transform layout is supported");
    ensure!(!r.bit()?, "fast chroma motion is not supported");
    let extended_mv = r.bit()?;
    ensure!(r.bits(2)? == 0, "macroblock quantizers are not supported");
    let vstransform = r.bit()?;
    ensure!(!r.bit()?, "transform table switching is not supported");
    ensure!(!r.bit()?, "overlap smoothing is not supported");
    ensure!(!r.bit()?, "resync markers are not supported");
    ensure!(!r.bit()?, "range reduction is not supported");
    ensure!(r.bits(3)? == 0, "B frames are not supported");
    ensure!(r.bits(2)? == 0, "only the implicit quantizer mode is supported");
    ensure!(!r.bit()?, "frame interpolation hints are not supported");
    ensure!(r.bit()?, "only the reduced 4MV predictor layout is supported");
    Ok(Sequence { loop_filter, extended_mv, vstransform })
}

/// The prefix codes, built once.
struct Codes {
    imode: Vlc,
    norm2: Vlc,
    norm6: Vlc,
    mb_i: Vlc,
    /// `[set * 2 + chroma]`.
    dc: Vec<Vlc>,
    ac: Vec<Vlc>,
    cbpcy: Vec<Vlc>,
    ttmb: Vec<Vlc>,
    ttblk: Vec<Vlc>,
    subblkpat: Vec<Vlc>,
    mv_diff: Vec<Vlc>,
}

fn vlc(codes: &[u32], bits: &[u8]) -> Result<Vlc> {
    Vlc::new(codes.iter().copied().zip(bits.iter().copied()))
}

fn vlc_rows(codes: &[u32], bits: &[u8], rows: usize) -> Result<Vec<Vlc>> {
    let n = codes.len() / rows;
    (0..rows).map(|i| vlc(&codes[i * n..(i + 1) * n], &bits[i * n..(i + 1) * n])).collect()
}

impl Codes {
    fn new() -> Result<Self> {
        let ac = (0..8)
            .map(|s| {
                let n = AC_SIZES[s] as usize;
                vlc(&AC_CODES[s * 186..s * 186 + n], &AC_BITS[s * 186..s * 186 + n])
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            imode: vlc(&IMODE_CODES, &IMODE_BITS)?,
            norm2: vlc(&NORM2_CODES, &NORM2_BITS)?,
            norm6: vlc(&NORM6_CODES, &NORM6_BITS)?,
            mb_i: vlc(&MB_I_CODES, &MB_I_BITS)?,
            dc: vlc_rows(&DC_CODES, &DC_BITS, 4)?,
            ac,
            cbpcy: vlc_rows(&CBPCY_P_CODES, &CBPCY_P_BITS, 4)?,
            ttmb: vlc_rows(&TTMB_CODES, &TTMB_BITS, 3)?,
            ttblk: vlc_rows(&TTBLK_CODES, &TTBLK_BITS, 3)?,
            subblkpat: vlc_rows(&SUBBLKPAT_CODES, &SUBBLKPAT_BITS, 3)?,
            mv_diff: vlc_rows(&MV_DIFF_CODES, &MV_DIFF_BITS, 4)?,
        })
    }
}

// Motion vector modes.
const MV_1MV_HPEL_BILIN: u8 = 0;
const MV_1MV_HPEL: u8 = 2;
const MV_MIXED: u8 = 3;
const MV_INTENSITY: u8 = 4;

// Transform types.
const TT_8X8: i32 = 0;
const TT_8X4_BOTTOM: i32 = 1;
const TT_8X4_TOP: i32 = 2;
const TT_8X4: i32 = 3;
const TT_4X8_RIGHT: i32 = 4;
const TT_4X8_LEFT: i32 = 5;
const TT_4X8: i32 = 6;
const TT_4X4: i32 = 7;

// Coefficient coding sets.
const CS_HIGH_MOT_INTRA: usize = 0;
const CS_HIGH_MOT_INTER: usize = 1;
const CS_LOW_MOT_INTRA: usize = 2;
const CS_LOW_MOT_INTER: usize = 3;
const CS_MID_RATE_INTRA: usize = 4;
const CS_MID_RATE_INTER: usize = 5;
const CS_HIGH_RATE_INTRA: usize = 6;
const CS_HIGH_RATE_INTER: usize = 7;

/// Per-frame header values.
struct Frame {
    p: bool,
    pq: i32,
    pqindex: u32,
    halfpq: i32,
    /// Uniform quantizer (no ±pq reconstruction offset).
    pquantizer: bool,
    k_x: u32,
    k_y: u32,
    range_x: i32,
    range_y: i32,
    tt_index: usize,
    quarter: bool,
    /// Bicubic interpolation (else half-sample bilinear).
    mspel: bool,
    /// Intensity compensation tables (luma, chroma).
    lut: Option<Box<([u8; 256], [u8; 256])>>,
    mv_type: Vec<u8>,
    mv_type_raw: bool,
    skip: Vec<u8>,
    skip_raw: bool,
    mv_table: usize,
    cbptab: usize,
    ttmbf: bool,
    ttfrm: i32,
    c_ac: u32,
    y_ac: u32,
    dc_table: usize,
}

/// Count bits until one equal to `stop` (consumed) or `len` bits were read.
fn unary(r: &mut BitReader<'_>, stop: bool, len: u32) -> Result<u32> {
    let mut i = 0;
    while i < len && r.bit()? != stop {
        i += 1;
    }
    Ok(i)
}

/// "0" → 0, "10" → 1, "11" → 2.
fn decode012(r: &mut BitReader<'_>) -> Result<u32> {
    Ok(if !r.bit()? { 0 } else { 1 + r.bit()? as u32 })
}

/// "1" → 0, "01" → 1, "00" → 2.
fn decode210(r: &mut BitReader<'_>) -> Result<u32> {
    Ok(if r.bit()? { 0 } else { 2 - r.bit()? as u32 })
}

/// Intensity compensation tables from the frame's scale and shift.
fn intensity_luts(lumscale: i32, lumshift: i32) -> ([u8; 256], [u8; 256]) {
    let (scale, shift) = if lumscale == 0 {
        let mut shift = (255 - lumshift * 2) * 64;
        if lumshift > 31 {
            shift += 128 << 6;
        }
        (-64, shift)
    } else {
        (lumscale + 32, if lumshift > 31 { (lumshift - 64) * 64 } else { lumshift << 6 })
    };
    let mut y = [0u8; 256];
    let mut uv = [0u8; 256];
    for i in 0..256 {
        y[i] = ((scale * i as i32 + shift + 32) >> 6).clamp(0, 255) as u8;
        uv[i] = ((scale * (i as i32 - 128) + 128 * 64 + 32) >> 6).clamp(0, 255) as u8;
    }
    (y, uv)
}

/// Decoding state rebuilt every frame (prediction neighbours and loop-filter data).
struct State {
    /// Block grid widths per plane: luma 8x8 blocks, chroma macroblocks.
    gw: [usize; 3],
    dc: [Vec<i32>; 3],
    /// Saved AC predictors: `[1..8]` the first column set, `[9..16]` the first row set.
    ac: [Vec<[i16; 16]>; 3],
    intra: [Vec<bool>; 3],
    coded: Vec<u8>,
    /// Motion vector per luma block (intra: zero).
    mv: Vec<[i32; 2]>,
    /// Per macroblock: chroma vector, coded 4x4 pattern (4 bits per block), transform types (4 bits per block),
    /// intra blocks (bit per block).
    cmv: Vec<[i32; 2]>,
    cbp: Vec<u32>,
    tt: Vec<u32>,
    intra_bits: Vec<u8>,
}

impl State {
    fn new(mbw: usize, mbh: usize) -> Self {
        let nl = 4 * mbw * mbh;
        let nc = mbw * mbh;
        Self {
            gw: [2 * mbw, mbw, mbw],
            dc: [vec![0; nl], vec![0; nc], vec![0; nc]],
            ac: [vec![[0; 16]; nl], vec![[0; 16]; nc], vec![[0; 16]; nc]],
            intra: [vec![false; nl], vec![false; nc], vec![false; nc]],
            coded: vec![0; nl],
            mv: vec![[0; 2]; nl],
            cmv: vec![[0; 2]; nc],
            cbp: vec![0; nc],
            tt: vec![0; nc],
            intra_bits: vec![0; nc],
        }
    }

    fn reset(&mut self) {
        for p in 0..3 {
            self.dc[p].fill(0);
            self.ac[p].fill([0; 16]);
            self.intra[p].fill(false);
        }
        self.coded.fill(0);
        self.mv.fill([0; 2]);
        self.cmv.fill([0; 2]);
        self.cbp.fill(0);
        self.tt.fill(0);
        self.intra_bits.fill(0);
    }
}

/// Block `n` (0..4 luma, 4 Cb, 5 Cr) of macroblock (`mx`, `my`): plane and position on that plane's block grid.
fn blk(n: usize, mx: usize, my: usize) -> (usize, usize, usize) {
    if n < 4 { (0, 2 * mx + (n & 1), 2 * my + (n >> 1)) } else { (n - 3, mx, my) }
}

/// Top-left sample of block `n` of macroblock (`mx`, `my`) on its plane.
fn blk_origin(n: usize, mx: usize, my: usize) -> (usize, usize) {
    if n < 4 { (mx * 16 + (n & 1) * 8, my * 16 + (n >> 1) * 8) } else { (mx * 8, my * 8) }
}

/// Decodes a WMV3 stream frame by frame.
pub struct Vc1Decoder {
    seq: Sequence,
    mbw: usize,
    mbh: usize,
    codes: Codes,
    cur: Picture,
    reference: Picture,
    have_reference: bool,
    rnd: i32,
    mvrange: u32,
    state: State,
}

impl std::fmt::Debug for Vc1Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vc1Decoder")
            .field("macroblocks", &(self.mbw, self.mbh))
            .field("seq", &self.seq)
            .finish()
    }
}

impl Vc1Decoder {
    pub fn new(format: &VideoFormat) -> Result<Self> {
        ensure!(&format.fourcc == b"WMV3", "video codec {:?} is not WMV3", format.fourcc);
        let seq = parse_sequence(&format.extra)?;
        let (w, h) = (format.width as usize, format.height as usize);
        ensure!(w > 0 && h > 0 && w <= 4096 && h <= 4096, "picture size {w}x{h}");
        let (mbw, mbh) = (w.div_ceil(16), h.div_ceil(16));
        Ok(Self {
            seq,
            mbw,
            mbh,
            codes: Codes::new()?,
            cur: Picture::new(mbw, mbh, w, h),
            reference: Picture::new(mbw, mbh, w, h),
            have_reference: false,
            rnd: 0,
            mvrange: 0,
            state: State::new(mbw, mbh),
        })
    }

    /// Decode one frame (one video object of the container). An empty frame repeats the previous picture.
    pub fn decode(&mut self, data: &[u8]) -> Result<&Picture> {
        let mut data = data;
        if data.len() >= 4 && data[data.len() - 4..] == [0, 0, 1, 0x0A] {
            data = &data[..data.len() - 4]; // end-of-sequence code
        }
        if data.is_empty() {
            ensure!(self.have_reference, "empty first frame");
            return Ok(&self.reference);
        }
        let mut r = BitReader::new(data);
        let f = self.parse_header(&mut r)?;
        ensure!(!f.p || self.have_reference, "P frame without a reference");
        self.state.reset();
        let mut ctx = blocks::Ctx {
            r,
            f: &f,
            seq: self.seq,
            codes: &self.codes,
            st: &mut self.state,
            cur: &mut self.cur,
            refp: &self.reference,
            mbw: self.mbw,
            mbh: self.mbh,
            rnd: self.rnd,
            esc3_level: 0,
            esc3_run: 0,
            codingset: 0,
            codingset2: 0,
        };
        if f.p {
            ctx.p_frame().context("P frame")?;
        } else {
            ctx.i_frame().context("I frame")?;
        }
        std::mem::swap(&mut self.cur, &mut self.reference);
        self.have_reference = true;
        Ok(&self.reference)
    }

    fn parse_header(&mut self, r: &mut BitReader<'_>) -> Result<Frame> {
        r.skip(2)?; // frame counter
        let p = r.bit()?;
        if !p {
            r.skip(7)?; // buffer fullness
            self.rnd = 1;
        } else {
            self.rnd ^= 1;
        }
        let pqindex = r.bits(5)?;
        ensure!(pqindex != 0, "picture quantizer index 0");
        let pq = PQUANT[pqindex as usize] as i32;
        let halfpq = if pqindex < 9 { r.bit()? as i32 } else { 0 };
        let pquantizer = pqindex < 9;
        if self.seq.extended_mv {
            self.mvrange = unary(r, false, 3)?;
        }
        let k_x = self.mvrange + 9 + (self.mvrange >> 1);
        let k_y = self.mvrange + 8;
        let mut f = Frame {
            p,
            pq,
            pqindex,
            halfpq,
            pquantizer,
            k_x,
            k_y,
            range_x: 1 << (k_x - 1),
            range_y: 1 << (k_y - 1),
            tt_index: 0,
            quarter: false,
            mspel: false,
            lut: None,
            mv_type: Vec::new(),
            mv_type_raw: false,
            skip: Vec::new(),
            skip_raw: false,
            mv_table: 0,
            cbptab: 0,
            ttmbf: true,
            ttfrm: TT_8X8,
            c_ac: 0,
            y_ac: 0,
            dc_table: 0,
        };
        if p {
            f.tt_index = (pq > 4) as usize + (pq > 12) as usize;
            let lowquant = if pq > 12 { 0 } else { 1 };
            let mut mode = MV_PMODE[lowquant * 5 + unary(r, true, 4)? as usize];
            let mut mixed = mode == MV_MIXED;
            if mode == MV_INTENSITY {
                mode = MV_PMODE2[lowquant * 4 + unary(r, true, 3)? as usize];
                mixed = mode == MV_MIXED;
                let lumscale = r.bits(6)? as i32;
                let lumshift = r.bits(6)? as i32;
                f.lut = Some(Box::new(intensity_luts(lumscale, lumshift)));
            }
            f.quarter = mode != MV_1MV_HPEL && mode != MV_1MV_HPEL_BILIN;
            f.mspel = mode != MV_1MV_HPEL_BILIN;
            let n = self.mbw * self.mbh;
            if mixed {
                (f.mv_type, f.mv_type_raw) = self.bitplane(r)?;
            } else {
                f.mv_type = vec![0; n];
            }
            (f.skip, f.skip_raw) = self.bitplane(r)?;
            f.mv_table = r.bits(2)? as usize;
            f.cbptab = r.bits(2)? as usize;
            if self.seq.vstransform {
                f.ttmbf = r.bit()?;
                f.ttfrm = if f.ttmbf { TTFRM_TO_TT[r.bits(2)? as usize] as i32 } else { TT_8X8 };
            }
        }
        f.c_ac = decode012(r)?;
        if !p {
            f.y_ac = decode012(r)?;
        }
        f.dc_table = r.bit()? as usize;
        Ok(f)
    }

    /// A per-macroblock flag plane. Returns the values and whether they are instead sent per macroblock.
    fn bitplane(&self, r: &mut BitReader<'_>) -> Result<(Vec<u8>, bool)> {
        let (w, h) = (self.mbw, self.mbh);
        let mut p = vec![0u8; w * h];
        let invert = r.bit()? as u8;
        let imode = self.codes.imode.read(r)?;
        fn rowskip(
            r: &mut BitReader<'_>,
            p: &mut [u8],
            w: usize,
            x0: usize,
            y0: usize,
            cols: usize,
            rows: usize,
        ) -> Result<()> {
            for y in y0..y0 + rows {
                let coded = r.bit()?;
                for x in x0..x0 + cols {
                    p[y * w + x] = if coded { r.bit()? as u8 } else { 0 };
                }
            }
            Ok(())
        }
        fn colskip(r: &mut BitReader<'_>, p: &mut [u8], w: usize, cols: usize, rows: usize) -> Result<()> {
            for x in 0..cols {
                let coded = r.bit()?;
                for y in 0..rows {
                    p[y * w + x] = if coded { r.bit()? as u8 } else { 0 };
                }
            }
            Ok(())
        }
        match imode {
            0 => return Ok((p, true)),
            1 | 2 => {
                let mut k = 0;
                if (w * h) & 1 == 1 {
                    p[0] = r.bit()? as u8;
                    k = 1;
                }
                while k < w * h {
                    let code = self.codes.norm2.read(r)? as u8;
                    p[k] = code & 1;
                    p[k + 1] = code >> 1;
                    k += 2;
                }
            }
            3 | 4 => {
                if h % 3 == 0 && w % 3 != 0 {
                    for y in (0..h).step_by(3) {
                        for x in ((w & 1)..w).step_by(2) {
                            let c = self.codes.norm6.read(r)? as u8;
                            for k in 0..6 {
                                p[(y + k / 2) * w + x + k % 2] = (c >> k) & 1;
                            }
                        }
                    }
                    if w & 1 == 1 {
                        colskip(r, &mut p, w, 1, h)?;
                    }
                } else {
                    for y in ((h & 1)..h).step_by(2) {
                        for x in ((w % 3)..w).step_by(3) {
                            let c = self.codes.norm6.read(r)? as u8;
                            for k in 0..6 {
                                p[(y + k / 3) * w + x + k % 3] = (c >> k) & 1;
                            }
                        }
                    }
                    let x0 = w % 3;
                    if x0 > 0 {
                        colskip(r, &mut p, w, x0, h)?;
                    }
                    if h & 1 == 1 {
                        rowskip(r, &mut p, w, x0, 0, w - x0, 1)?;
                    }
                }
            }
            5 => rowskip(r, &mut p, w, 0, 0, w, h)?,
            6 => colskip(r, &mut p, w, w, h)?,
            m => bail!("bitplane mode {m}"),
        }
        if imode == 2 || imode == 4 {
            p[0] ^= invert;
            for x in 1..w {
                p[x] ^= p[x - 1];
            }
            for y in 1..h {
                p[y * w] ^= p[(y - 1) * w];
                for x in 1..w {
                    let i = y * w + x;
                    if p[i - 1] != p[i - w] {
                        p[i] ^= invert;
                    } else {
                        p[i] ^= p[i - 1];
                    }
                }
            }
        } else if invert != 0 {
            for v in &mut p {
                *v ^= 1;
            }
        }
        Ok((p, false))
    }
}
