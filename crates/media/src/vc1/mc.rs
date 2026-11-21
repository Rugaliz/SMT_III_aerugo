//! Motion compensation: predicted blocks from the reference picture.
//!
//! Reads outside the picture repeat the nearest edge pixel. With intensity compensation, every reference pixel
//! read goes through the frame's lookup table first.

use super::Plane;
use super::dsp;

/// A copy of a reference area with edge pixels repeated, optionally remapped by a lookup table.
fn patch(src: &Plane, x0: i32, y0: i32, w: usize, h: usize, lut: Option<&[u8; 256]>) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    let (pw, ph) = (src.width as i32, src.height as i32);
    for j in 0..h {
        let y = (y0 + j as i32).clamp(0, ph - 1) as usize;
        let row = &src.data[y * src.width..(y + 1) * src.width];
        for i in 0..w {
            let x = (x0 + i as i32).clamp(0, pw - 1) as usize;
            let v = row[x];
            out[j * w + i] = match lut {
                Some(l) => l[v as usize],
                None => v,
            };
        }
    }
    out
}

/// Predict an `n` x `n` luma block at (`dx`, `dy`) of `dst` from reference position (`sx`, `sy`) with
/// quarter-sample fractions `fx`, `fy`. `bicubic`: the 4-tap filters (else half-sample bilinear).
#[allow(clippy::too_many_arguments)]
pub(crate) fn luma(
    dst: &mut Plane,
    dx: usize,
    dy: usize,
    src: &Plane,
    sx: i32,
    sy: i32,
    fx: i32,
    fy: i32,
    n: usize,
    bicubic: bool,
    rnd: i32,
    lut: Option<&[u8; 256]>,
) {
    let w = n + 3;
    let p = patch(src, sx - 1, sy - 1, w, w, lut);
    let dp = dy * dst.width + dx;
    let ds = dst.width;
    if bicubic {
        dsp::bicubic_mc(&mut dst.data, dp, ds, &p, w + 1, w, n, fx as u32, fy as u32, rnd);
        return;
    }
    // Half-sample bilinear: dxy bit 0 = horizontal half, bit 1 = vertical half.
    let hx = (fx & 2) != 0;
    let hy = (fy & 2) != 0;
    let at = |i: usize, j: usize| p[(j + 1) * w + i + 1] as u32;
    for j in 0..n {
        for i in 0..n {
            let v = match (hx, hy) {
                (false, false) => at(i, j),
                (true, false) => {
                    let s = at(i, j) + at(i + 1, j);
                    if rnd == 0 { (s + 1) >> 1 } else { s >> 1 }
                }
                (false, true) => {
                    let s = at(i, j) + at(i, j + 1);
                    if rnd == 0 { (s + 1) >> 1 } else { s >> 1 }
                }
                (true, true) => {
                    let s = at(i, j) + at(i + 1, j) + at(i, j + 1) + at(i + 1, j + 1);
                    if rnd == 0 { (s + 2) >> 2 } else { (s + 1) >> 2 }
                }
            };
            dst.data[dp + j * ds + i] = v as u8;
        }
    }
}

/// Predict an 8x8 chroma block at (`dx`, `dy`) from (`sx`, `sy`) with eighth-sample fractions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chroma(
    dst: &mut Plane,
    dx: usize,
    dy: usize,
    src: &Plane,
    sx: i32,
    sy: i32,
    fx: i32,
    fy: i32,
    rnd: i32,
    lut: Option<&[u8; 256]>,
) {
    let p = patch(src, sx, sy, 9, 9, lut);
    let dp = dy * dst.width + dx;
    let ds = dst.width;
    dsp::chroma_mc(&mut dst.data, dp, ds, &p, 0, 9, 8, 8, fx, fy, rnd == 0);
}
