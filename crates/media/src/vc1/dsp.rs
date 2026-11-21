//! Signal processing of the WMV3 video format: inverse transforms, the in-loop deblocking filter and the
//! sub-pixel interpolation filters. The arithmetic (rounding offsets and shifts) follows the format exactly, so
//! decoded pictures are identical to the reference.
//!
//! Coefficient blocks are 64 values in the transposed layout the scan tables produce.

#[inline]
fn clip8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

// ---- deblocking ----------------------------------------------------------------------------------

/// Filter across one edge position. `p` is the index of the first pixel past the edge, `s` the step across the
/// edge. Returns whether the line was considered for filtering (used to decide the other lines of a segment).
fn filter_line(buf: &mut [u8], p: usize, s: usize, pq: i32) -> bool {
    let at = |k: isize| buf[(p as isize + k * s as isize) as usize] as i32;
    let mut a0 = (2 * (at(-2) - at(1)) - 5 * (at(-1) - at(0)) + 4) >> 3;
    let a0_sign = a0 >> 31;
    a0 = (a0 ^ a0_sign) - a0_sign;
    if a0 < pq {
        let a1 = ((2 * (at(-4) - at(-1)) - 5 * (at(-3) - at(-2)) + 4) >> 3).abs();
        let a2 = ((2 * (at(0) - at(3)) - 5 * (at(1) - at(2)) + 4) >> 3).abs();
        if a1 < a0 || a2 < a0 {
            let mut clip = at(-1) - at(0);
            let clip_sign = clip >> 31;
            clip = ((clip ^ clip_sign) - clip_sign) >> 1;
            if clip != 0 {
                let a3 = a1.min(a2);
                let mut d = (5 * (a0 - a3)) >> 3;
                if (a0_sign ^ clip_sign) != 0 {
                    d = d.min(clip);
                    d = (d ^ clip_sign) - clip_sign;
                    let i = p - s;
                    buf[i] = clip8(buf[i] as i32 - d);
                    buf[p] = clip8(buf[p] as i32 + d);
                }
                return true;
            }
        }
    }
    false
}

/// Filter `len` pixels along an edge, in groups of 4 decided by the third line of each group.
/// `p`: first pixel past the edge; `along`: step along the edge; `across`: step across it.
pub(crate) fn loop_filter(buf: &mut [u8], mut p: usize, along: usize, across: usize, len: usize, pq: i32) {
    for _ in (0..len).step_by(4) {
        if filter_line(buf, p + 2 * along, across, pq) {
            filter_line(buf, p, across, pq);
            filter_line(buf, p + along, across, pq);
            filter_line(buf, p + 3 * along, across, pq);
        }
        p += 4 * along;
    }
}

/// Filter a horizontal edge (between rows) at `p` (first pixel of the lower row), `len` pixels wide.
pub(crate) fn filter_horizontal_edge(buf: &mut [u8], p: usize, stride: usize, len: usize, pq: i32) {
    loop_filter(buf, p, 1, stride, len, pq);
}

/// Filter a vertical edge (between columns) at `p` (first pixel of the right column), `len` pixels tall.
pub(crate) fn filter_vertical_edge(buf: &mut [u8], p: usize, stride: usize, len: usize, pq: i32) {
    loop_filter(buf, p, stride, 1, len, pq);
}

// ---- inverse transforms --------------------------------------------------------------------------

/// 8x8 inverse transform in place (intra blocks: output values, not yet clamped).
pub(crate) fn inv_8x8(block: &mut [i16; 64]) {
    let mut tmp = [0i16; 64];
    for i in 0..8 {
        let s = |k: usize| block[i + k] as i32;
        let t1 = 12 * (s(0) + s(32)) + 4;
        let t2 = 12 * (s(0) - s(32)) + 4;
        let t3 = 16 * s(16) + 6 * s(48);
        let t4 = 6 * s(16) - 16 * s(48);
        let (t5, t6, t7, t8) = (t1 + t3, t2 + t4, t2 - t4, t1 - t3);
        let t1 = 16 * s(8) + 15 * s(24) + 9 * s(40) + 4 * s(56);
        let t2 = 15 * s(8) - 4 * s(24) - 16 * s(40) - 9 * s(56);
        let t3 = 9 * s(8) - 16 * s(24) + 4 * s(40) + 15 * s(56);
        let t4 = 4 * s(8) - 9 * s(24) + 15 * s(40) - 16 * s(56);
        let d = &mut tmp[i * 8..i * 8 + 8];
        d[0] = ((t5 + t1) >> 3) as i16;
        d[1] = ((t6 + t2) >> 3) as i16;
        d[2] = ((t7 + t3) >> 3) as i16;
        d[3] = ((t8 + t4) >> 3) as i16;
        d[4] = ((t8 - t4) >> 3) as i16;
        d[5] = ((t7 - t3) >> 3) as i16;
        d[6] = ((t6 - t2) >> 3) as i16;
        d[7] = ((t5 - t1) >> 3) as i16;
    }
    for i in 0..8 {
        let s = |k: usize| tmp[i + k] as i32;
        let t1 = 12 * (s(0) + s(32)) + 64;
        let t2 = 12 * (s(0) - s(32)) + 64;
        let t3 = 16 * s(16) + 6 * s(48);
        let t4 = 6 * s(16) - 16 * s(48);
        let (t5, t6, t7, t8) = (t1 + t3, t2 + t4, t2 - t4, t1 - t3);
        let t1 = 16 * s(8) + 15 * s(24) + 9 * s(40) + 4 * s(56);
        let t2 = 15 * s(8) - 4 * s(24) - 16 * s(40) - 9 * s(56);
        let t3 = 9 * s(8) - 16 * s(24) + 4 * s(40) + 15 * s(56);
        let t4 = 4 * s(8) - 9 * s(24) + 15 * s(40) - 16 * s(56);
        block[i] = ((t5 + t1) >> 7) as i16;
        block[i + 8] = ((t6 + t2) >> 7) as i16;
        block[i + 16] = ((t7 + t3) >> 7) as i16;
        block[i + 24] = ((t8 + t4) >> 7) as i16;
        block[i + 32] = ((t8 - t4 + 1) >> 7) as i16;
        block[i + 40] = ((t7 - t3 + 1) >> 7) as i16;
        block[i + 48] = ((t6 - t2 + 1) >> 7) as i16;
        block[i + 56] = ((t5 - t1 + 1) >> 7) as i16;
    }
}

/// Write an 8x8 block of values, clamped.
pub(crate) fn put_clamped(block: &[i16; 64], dst: &mut [u8], p: usize, stride: usize) {
    for y in 0..8 {
        for x in 0..8 {
            dst[p + y * stride + x] = clip8(block[y * 8 + x] as i32);
        }
    }
}

/// Write an 8x8 block of signed values around mid-grey (+128), clamped.
pub(crate) fn put_signed_clamped(block: &[i16; 64], dst: &mut [u8], p: usize, stride: usize) {
    for y in 0..8 {
        for x in 0..8 {
            dst[p + y * stride + x] = clip8(block[y * 8 + x] as i32 + 128);
        }
    }
}

/// Add an 8x8 block of residuals, clamped.
pub(crate) fn add_clamped(block: &[i16; 64], dst: &mut [u8], p: usize, stride: usize) {
    for y in 0..8 {
        for x in 0..8 {
            let i = p + y * stride + x;
            dst[i] = clip8(dst[i] as i32 + block[y * 8 + x] as i32);
        }
    }
}

fn add_dc(dst: &mut [u8], p: usize, stride: usize, w: usize, h: usize, dc: i32) {
    for y in 0..h {
        for x in 0..w {
            let i = p + y * stride + x;
            dst[i] = clip8(dst[i] as i32 + dc);
        }
    }
}

/// 8x8 residual with only a DC coefficient, added.
pub(crate) fn inv_8x8_dc_add(dst: &mut [u8], p: usize, stride: usize, block: &[i16; 64]) {
    let dc = block[0] as i32;
    let dc = (3 * dc + 1) >> 1;
    let dc = (3 * dc + 16) >> 5;
    add_dc(dst, p, stride, 8, 8, dc);
}

/// 8 wide, 4 tall residual (coefficients in rows of 8), added.
pub(crate) fn inv_8x4_add(dst: &mut [u8], p: usize, stride: usize, block: &mut [i16; 64]) {
    for i in 0..4 {
        let s = |k: usize| block[i * 8 + k] as i32;
        let t1 = 12 * (s(0) + s(4)) + 4;
        let t2 = 12 * (s(0) - s(4)) + 4;
        let t3 = 16 * s(2) + 6 * s(6);
        let t4 = 6 * s(2) - 16 * s(6);
        let (t5, t6, t7, t8) = (t1 + t3, t2 + t4, t2 - t4, t1 - t3);
        let t1 = 16 * s(1) + 15 * s(3) + 9 * s(5) + 4 * s(7);
        let t2 = 15 * s(1) - 4 * s(3) - 16 * s(5) - 9 * s(7);
        let t3 = 9 * s(1) - 16 * s(3) + 4 * s(5) + 15 * s(7);
        let t4 = 4 * s(1) - 9 * s(3) + 15 * s(5) - 16 * s(7);
        let d = &mut block[i * 8..i * 8 + 8];
        d[0] = ((t5 + t1) >> 3) as i16;
        d[1] = ((t6 + t2) >> 3) as i16;
        d[2] = ((t7 + t3) >> 3) as i16;
        d[3] = ((t8 + t4) >> 3) as i16;
        d[4] = ((t8 - t4) >> 3) as i16;
        d[5] = ((t7 - t3) >> 3) as i16;
        d[6] = ((t6 - t2) >> 3) as i16;
        d[7] = ((t5 - t1) >> 3) as i16;
    }
    for i in 0..8 {
        let s = |k: usize| block[i + k] as i32;
        let t1 = 17 * (s(0) + s(16)) + 64;
        let t2 = 17 * (s(0) - s(16)) + 64;
        let t3 = 22 * s(8) + 10 * s(24);
        let t4 = 22 * s(24) - 10 * s(8);
        let o = [(t1 + t3) >> 7, (t2 - t4) >> 7, (t2 + t4) >> 7, (t1 - t3) >> 7];
        for (r, v) in o.iter().enumerate() {
            let j = p + r * stride + i;
            dst[j] = clip8(dst[j] as i32 + v);
        }
    }
}

pub(crate) fn inv_8x4_dc_add(dst: &mut [u8], p: usize, stride: usize, block: &[i16; 64]) {
    let dc = block[0] as i32;
    let dc = (3 * dc + 1) >> 1;
    let dc = (17 * dc + 64) >> 7;
    add_dc(dst, p, stride, 8, 4, dc);
}

/// 4 wide, 8 tall residual (coefficients in rows of 8, 4 used), added.
pub(crate) fn inv_4x8_add(dst: &mut [u8], p: usize, stride: usize, block: &mut [i16; 64]) {
    for i in 0..8 {
        let s = |k: usize| block[i * 8 + k] as i32;
        let t1 = 17 * (s(0) + s(2)) + 4;
        let t2 = 17 * (s(0) - s(2)) + 4;
        let t3 = 22 * s(1) + 10 * s(3);
        let t4 = 22 * s(3) - 10 * s(1);
        let d = &mut block[i * 8..i * 8 + 4];
        d[0] = ((t1 + t3) >> 3) as i16;
        d[1] = ((t2 - t4) >> 3) as i16;
        d[2] = ((t2 + t4) >> 3) as i16;
        d[3] = ((t1 - t3) >> 3) as i16;
    }
    for i in 0..4 {
        let s = |k: usize| block[i + k] as i32;
        let t1 = 12 * (s(0) + s(32)) + 64;
        let t2 = 12 * (s(0) - s(32)) + 64;
        let t3 = 16 * s(16) + 6 * s(48);
        let t4 = 6 * s(16) - 16 * s(48);
        let (t5, t6, t7, t8) = (t1 + t3, t2 + t4, t2 - t4, t1 - t3);
        let t1 = 16 * s(8) + 15 * s(24) + 9 * s(40) + 4 * s(56);
        let t2 = 15 * s(8) - 4 * s(24) - 16 * s(40) - 9 * s(56);
        let t3 = 9 * s(8) - 16 * s(24) + 4 * s(40) + 15 * s(56);
        let t4 = 4 * s(8) - 9 * s(24) + 15 * s(40) - 16 * s(56);
        let o = [
            (t5 + t1) >> 7,
            (t6 + t2) >> 7,
            (t7 + t3) >> 7,
            (t8 + t4) >> 7,
            (t8 - t4 + 1) >> 7,
            (t7 - t3 + 1) >> 7,
            (t6 - t2 + 1) >> 7,
            (t5 - t1 + 1) >> 7,
        ];
        for (r, v) in o.iter().enumerate() {
            let j = p + r * stride + i;
            dst[j] = clip8(dst[j] as i32 + v);
        }
    }
}

pub(crate) fn inv_4x8_dc_add(dst: &mut [u8], p: usize, stride: usize, block: &[i16; 64]) {
    let dc = block[0] as i32;
    let dc = (17 * dc + 4) >> 3;
    let dc = (12 * dc + 64) >> 7;
    add_dc(dst, p, stride, 4, 8, dc);
}

/// 4x4 residual (coefficients in rows of 8), added.
pub(crate) fn inv_4x4_add(dst: &mut [u8], p: usize, stride: usize, block: &mut [i16; 64]) {
    for i in 0..4 {
        let s = |k: usize| block[i * 8 + k] as i32;
        let t1 = 17 * (s(0) + s(2)) + 4;
        let t2 = 17 * (s(0) - s(2)) + 4;
        let t3 = 22 * s(1) + 10 * s(3);
        let t4 = 22 * s(3) - 10 * s(1);
        let d = &mut block[i * 8..i * 8 + 4];
        d[0] = ((t1 + t3) >> 3) as i16;
        d[1] = ((t2 - t4) >> 3) as i16;
        d[2] = ((t2 + t4) >> 3) as i16;
        d[3] = ((t1 - t3) >> 3) as i16;
    }
    for i in 0..4 {
        let s = |k: usize| block[i + k] as i32;
        let t1 = 17 * (s(0) + s(16)) + 64;
        let t2 = 17 * (s(0) - s(16)) + 64;
        let t3 = 22 * s(8) + 10 * s(24);
        let t4 = 22 * s(24) - 10 * s(8);
        let o = [(t1 + t3) >> 7, (t2 - t4) >> 7, (t2 + t4) >> 7, (t1 - t3) >> 7];
        for (r, v) in o.iter().enumerate() {
            let j = p + r * stride + i;
            dst[j] = clip8(dst[j] as i32 + v);
        }
    }
}

pub(crate) fn inv_4x4_dc_add(dst: &mut [u8], p: usize, stride: usize, block: &[i16; 64]) {
    let dc = block[0] as i32;
    let dc = (17 * dc + 4) >> 3;
    let dc = (17 * dc + 64) >> 7;
    add_dc(dst, p, stride, 4, 4, dc);
}

// ---- interpolation -------------------------------------------------------------------------------

/// The 4-tap bicubic filter without rounding (`mode` 1 = 1/4, 2 = 1/2, 3 = 3/4 sample).
fn bicubic_raw(at: impl Fn(isize) -> i32, mode: u32) -> i32 {
    match mode {
        1 => -4 * at(-1) + 53 * at(0) + 18 * at(1) - 3 * at(2),
        2 => -at(-1) + 9 * at(0) + 9 * at(1) - at(2),
        3 => -3 * at(-1) + 18 * at(0) + 53 * at(1) - 4 * at(2),
        _ => 0,
    }
}

fn bicubic_rounded(at: impl Fn(isize) -> i32, mode: u32, r: i32) -> i32 {
    match mode {
        0 => at(0),
        1 => (bicubic_raw(at, 1) + 32 - r) >> 6,
        2 => (bicubic_raw(at, 2) + 8 - r) >> 4,
        _ => (bicubic_raw(at, 3) + 32 - r) >> 6,
    }
}

/// Bicubic sub-pixel interpolation of an `n` x `n` block (n = 8 or 16) from `src` at `sp` (stride `ss`) into
/// `dst` at `dp` (stride `ds`). `hmode`/`vmode`: horizontal/vertical quarter positions; `rnd`: rounding control.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bicubic_mc(
    dst: &mut [u8],
    dp: usize,
    ds: usize,
    src: &[u8],
    sp: usize,
    ss: usize,
    n: usize,
    hmode: u32,
    vmode: u32,
    rnd: i32,
) {
    let px = |i: isize| src[(sp as isize + i) as usize] as i32;
    if vmode != 0 {
        if hmode != 0 {
            const SHIFT: [i32; 4] = [0, 5, 1, 5];
            let shift = (SHIFT[hmode as usize] + SHIFT[vmode as usize]) >> 1;
            let r = (1 << (shift - 1)) + rnd - 1;
            let w = n + 3;
            let mut tmp = vec![0i32; w * n];
            for j in 0..n {
                for i in 0..w {
                    let base = (j * ss) as isize + i as isize - 1;
                    let v = bicubic_raw(|k| px(base + k * ss as isize), vmode);
                    tmp[j * w + i] = (((v + r) >> shift) as i16) as i32;
                }
            }
            let r = 64 - rnd;
            for j in 0..n {
                for i in 0..n {
                    let t = |k: isize| tmp[(j * w) as isize as usize + (i as isize + 1 + k) as usize];
                    dst[dp + j * ds + i] = clip8((bicubic_raw(t, hmode) + r) >> 7);
                }
            }
        } else {
            let r = 1 - rnd;
            for j in 0..n {
                for i in 0..n {
                    let base = (j * ss + i) as isize;
                    dst[dp + j * ds + i] = clip8(bicubic_rounded(|k| px(base + k * ss as isize), vmode, r));
                }
            }
        }
        return;
    }
    for j in 0..n {
        for i in 0..n {
            let base = (j * ss + i) as isize;
            dst[dp + j * ds + i] = clip8(bicubic_rounded(|k| px(base + k), hmode, rnd));
        }
    }
}

/// Bilinear chroma interpolation in eighth samples (`x`, `y` in 0..8), `w` x `h`. `rounding`: true adds 32
/// (rounding mode), false adds 28 (the "no rounding" variant).
#[allow(clippy::too_many_arguments)]
pub(crate) fn chroma_mc(
    dst: &mut [u8],
    dp: usize,
    ds: usize,
    src: &[u8],
    sp: usize,
    ss: usize,
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    rounding: bool,
) {
    let a = (8 - x) * (8 - y);
    let b = x * (8 - y);
    let c = (8 - x) * y;
    let d = x * y;
    let bias = if rounding { 32 } else { 28 };
    for j in 0..h {
        for i in 0..w {
            let s = |o: usize| src[sp + j * ss + i + o] as i32;
            let v = (a * s(0) + b * s(1) + c * s(ss) + d * s(ss + 1) + bias) >> 6;
            dst[dp + j * ds + i] = v as u8;
        }
    }
}
