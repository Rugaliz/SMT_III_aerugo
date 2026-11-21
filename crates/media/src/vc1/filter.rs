//! The in-loop deblocking filter over a whole decoded picture.

use super::dsp::{filter_horizontal_edge, filter_vertical_edge};
use super::{Picture, State, TT_4X4, TT_4X8, TT_8X4, blk_origin};

/// I frames: every 8x8 block edge inside the picture, horizontal edges first. (Edges of one direction never
/// touch the same samples, so their order does not matter.)
pub(super) fn i_frame(pic: &mut Picture, pq: i32) {
    for p in [&mut pic.y, &mut pic.cb, &mut pic.cr] {
        let (w, h) = (p.width, p.height);
        for y in (8..h).step_by(8) {
            filter_horizontal_edge(&mut p.data, y * w, w, w, pq);
        }
        for x in (8..w).step_by(8) {
            filter_vertical_edge(&mut p.data, x, w, h, pq);
        }
    }
}

/// What the filter needs to know about one block.
struct Side {
    cbp: u32,
    intra: bool,
    mv: [i32; 2],
}

fn side(st: &State, mbw: usize, mb: usize, n: usize) -> Side {
    let (mx, my) = (mb % mbw, mb / mbw);
    let mv = if n < 4 { st.mv[(2 * my + (n >> 1)) * st.gw[0] + 2 * mx + (n & 1)] } else { st.cmv[mb] };
    Side { cbp: (st.cbp[mb] >> (4 * n)) & 0xF, intra: (st.intra_bits[mb] >> n) & 1 != 0, mv }
}

/// P frames: block edges are filtered where either side is intra, the motion differs, or the 4x4 sub-blocks
/// along the edge carry residuals; plus the internal edges of split transforms. All horizontal edges (in
/// macroblock and block order, each block's lower edge before its internal one), then all vertical edges.
pub(super) fn p_frame(pic: &mut Picture, st: &State, mbw: usize, mbh: usize, pq: i32) {
    for my in 0..mbh {
        for mx in 0..mbw {
            let mb = my * mbw + mx;
            for n in 0..6 {
                let (ox, oy) = blk_origin(n, mx, my);
                let p = match n {
                    0..4 => &mut pic.y,
                    4 => &mut pic.cb,
                    _ => &mut pic.cr,
                };
                let s = p.width;
                let this = side(st, mbw, mb, n);
                if !(my == mbh - 1 && n >= 2) {
                    let below = match n {
                        0 | 1 => side(st, mbw, mb, n + 2),
                        2 | 3 => side(st, mbw, mb + mbw, n - 2),
                        _ => side(st, mbw, mb + mbw, n),
                    };
                    let at = (oy + 8) * s + ox;
                    if this.intra || below.intra || this.mv != below.mv {
                        filter_horizontal_edge(&mut p.data, at, s, 8, pq);
                    } else {
                        let idx = (this.cbp | (below.cbp >> 2)) & 3;
                        if idx & 1 != 0 {
                            filter_horizontal_edge(&mut p.data, at + 4, s, 4, pq);
                        }
                        if idx & 2 != 0 {
                            filter_horizontal_edge(&mut p.data, at, s, 4, pq);
                        }
                    }
                }
                let tt = ((st.tt[mb] >> (4 * n)) & 0xF) as i32;
                if tt == TT_4X4 || tt == TT_8X4 {
                    let at = (oy + 4) * s + ox;
                    if this.cbp & 5 != 0 {
                        filter_horizontal_edge(&mut p.data, at + 4, s, 4, pq);
                    }
                    if this.cbp & 10 != 0 {
                        filter_horizontal_edge(&mut p.data, at, s, 4, pq);
                    }
                }
            }
        }
    }
    for my in 0..mbh {
        for mx in 0..mbw {
            let mb = my * mbw + mx;
            for n in 0..6 {
                let (ox, oy) = blk_origin(n, mx, my);
                let p = match n {
                    0..4 => &mut pic.y,
                    4 => &mut pic.cb,
                    _ => &mut pic.cr,
                };
                let s = p.width;
                let this = side(st, mbw, mb, n);
                if !(mx == mbw - 1 && n != 0 && n != 2) {
                    let right = match n {
                        0 | 2 => side(st, mbw, mb, n + 1),
                        1 | 3 => side(st, mbw, mb + 1, n - 1),
                        _ => side(st, mbw, mb + 1, n),
                    };
                    let at = oy * s + ox + 8;
                    if this.intra || right.intra || this.mv != right.mv {
                        filter_vertical_edge(&mut p.data, at, s, 8, pq);
                    } else {
                        let idx = (this.cbp | (right.cbp >> 1)) & 5;
                        if idx & 1 != 0 {
                            filter_vertical_edge(&mut p.data, at + 4 * s, s, 4, pq);
                        }
                        if idx & 4 != 0 {
                            filter_vertical_edge(&mut p.data, at, s, 4, pq);
                        }
                    }
                }
                let tt = ((st.tt[mb] >> (4 * n)) & 0xF) as i32;
                if tt == TT_4X4 || tt == TT_4X8 {
                    let at = oy * s + ox + 4;
                    if this.cbp & 3 != 0 {
                        filter_vertical_edge(&mut p.data, at + 4 * s, s, 4, pq);
                    }
                    if this.cbp & 12 != 0 {
                        filter_vertical_edge(&mut p.data, at, s, 4, pq);
                    }
                }
            }
        }
    }
}
