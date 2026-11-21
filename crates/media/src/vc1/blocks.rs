//! Macroblock and block decoding: coefficients, prediction of DC/AC values and motion vectors, and motion
//! compensation calls for I and P frames.

use super::*;

/// Default DC predictor by DC scale (picture edges in I frames).
const DCPRED: [i32; 32] = [
    -1, 1024, 512, 341, 256, 205, 171, 146, 128, 114, 102, 93, 85, 79, 73, 68, 64, 60, 57, 54, 51, 49, 47,
    45, 43, 41, 39, 38, 37, 35, 34, 33,
];

/// Motion vector difference: base offsets and extra bit counts by size class.
const MV_OFFSET: [i32; 6] = [0, 1, 3, 7, 15, 31];
const MV_SIZE: [u32; 6] = [0, 2, 3, 4, 5, 8];

/// 8x8 scan `t` (0..4) in the transposed coefficient layout.
#[inline]
fn zz(t: usize, i: usize) -> usize {
    let x = WMV1_SCAN[t * 64 + i] as usize;
    (x >> 3) | ((x & 7) << 3)
}

fn mid_pred(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

fn median4(a: i32, b: i32, c: i32, d: i32) -> i32 {
    if a < b {
        if c < d { (b.min(d) + a.max(c)) / 2 } else { (b.min(c) + a.max(d)) / 2 }
    } else if c < d {
        (a.min(d) + b.max(c)) / 2
    } else {
        (a.min(c) + b.max(d)) / 2
    }
}

/// Uniform/non-uniform reconstruction of one dequantized value (16-bit arithmetic, as stored).
#[inline]
fn requant(v: i16, scale: i32, q: i32, uniform: bool) -> i16 {
    let v = v.wrapping_mul(scale as i16);
    if uniform { v } else { v.wrapping_add(if v < 0 { -q } else { q } as i16) }
}

pub(super) struct Ctx<'a, 'b> {
    pub r: BitReader<'a>,
    pub f: &'b Frame,
    pub seq: Sequence,
    pub codes: &'b Codes,
    pub st: &'b mut State,
    pub cur: &'b mut Picture,
    pub refp: &'b Picture,
    pub mbw: usize,
    pub mbh: usize,
    pub rnd: i32,
    pub esc3_level: u32,
    pub esc3_run: u32,
    pub codingset: usize,
    pub codingset2: usize,
}

impl Ctx<'_, '_> {
    // ---- coefficients ----------------------------------------------------------------------------

    /// One AC symbol: (last, run, value).
    fn ac_symbol(&mut self, set: usize) -> Result<(bool, usize, i32)> {
        let size = AC_SIZES[set] as u32;
        let row = set * 185;
        let last_start = LAST_DECODE[set] as u32;
        let index = self.codes.ac[set].read(&mut self.r)?;
        let (last, run, level, sign);
        if index != size - 1 {
            run = INDEX_RUN[row + index as usize] as usize;
            level = INDEX_LEVEL[row + index as usize] as i32;
            last = index >= last_start;
            sign = self.r.bit()?;
        } else {
            let esc = decode210(&mut self.r)?;
            if esc != 2 {
                let i2 = self.codes.ac[set].read(&mut self.r)?;
                ensure!(i2 < size - 1, "escape inside an escaped AC symbol");
                let mut rn = INDEX_RUN[row + i2 as usize] as usize;
                let mut lv = INDEX_LEVEL[row + i2 as usize] as i32;
                let l = i2 >= last_start;
                let delta = |t: &[u8], width: usize, i: usize| -> Result<i32> {
                    ensure!(i < width, "AC escape delta index {i}");
                    Ok(t[set * width + i] as i32)
                };
                if esc == 0 {
                    lv += if l {
                        delta(&LAST_DELTA_LEVEL_TABLE, 44, rn)?
                    } else {
                        delta(&DELTA_LEVEL_TABLE, 31, rn)?
                    };
                } else {
                    let i = lv as usize;
                    rn += if l {
                        delta(&LAST_DELTA_RUN_TABLE, 10, i)?
                    } else {
                        delta(&DELTA_RUN_TABLE, 57, i)?
                    } as usize
                        + 1;
                }
                run = rn;
                level = lv;
                last = l;
                sign = self.r.bit()?;
            } else {
                last = self.r.bit()?;
                if self.esc3_level == 0 {
                    if self.f.pq < 8 {
                        self.esc3_level = self.r.bits(3)?;
                        if self.esc3_level == 0 {
                            self.esc3_level = self.r.bits(2)? + 8;
                        }
                    } else {
                        self.esc3_level = unary(&mut self.r, true, 6)? + 2;
                    }
                    self.esc3_run = 3 + self.r.bits(2)?;
                }
                run = self.r.bits(self.esc3_run)? as usize;
                sign = self.r.bit()?;
                level = self.r.bits(self.esc3_level)? as i32;
            }
        }
        Ok((last, run, if sign { -level } else { level }))
    }

    /// DC differential with quantizer `q`.
    fn dc_diff(&mut self, chroma: bool, q: i32) -> Result<i32> {
        let mut d = self.codes.dc[self.f.dc_table * 2 + chroma as usize].read(&mut self.r)? as i32;
        if d != 0 {
            let m = if q == 1 || q == 2 { (3 - q) as u32 } else { 0 };
            if d == 119 {
                d = self.r.bits(8 + m)? as i32;
            } else if m > 0 {
                d = (d << m) + self.r.bits(m)? as i32 - ((1 << m) - 1);
            }
            if self.r.bit()? {
                d = -d;
            }
        }
        Ok(d)
    }

    /// Read coefficients into `block` along scan `t` until the last one (or past position 63).
    fn read_intra_ac(&mut self, block: &mut [i16; 64], set: usize, t: usize) -> Result<()> {
        let mut i = 1usize;
        loop {
            let (last, run, value) = self.ac_symbol(set)?;
            i += run;
            if i > 63 {
                break;
            }
            block[zz(t, i)] = value as i16;
            i += 1;
            if last {
                break;
            }
        }
        Ok(())
    }

    /// Apply AC prediction, save this block's predictors, and dequantize (intra blocks).
    #[allow(clippy::too_many_arguments)]
    fn finish_intra(
        &mut self,
        block: &mut [i16; 64],
        pl: usize,
        idx: usize,
        coded: bool,
        use_pred: bool,
        left: bool,
        pset: &[i16; 16],
        q: i32,
    ) {
        let scale = 2 * q + self.f.halfpq;
        let uniform = self.f.pquantizer;
        let mut save = [0i16; 16];
        if coded {
            if use_pred {
                for k in 1..8 {
                    if left {
                        block[k] = block[k].wrapping_add(pset[k]);
                    } else {
                        block[k << 3] = block[k << 3].wrapping_add(pset[k + 8]);
                    }
                }
            }
            for k in 1..8 {
                save[k] = block[k];
                save[k + 8] = block[k << 3];
            }
            for v in block.iter_mut().skip(1) {
                if *v != 0 {
                    *v = requant(*v, scale, q, uniform);
                }
            }
        } else if use_pred {
            for k in 1..8 {
                let (pos, s) = if left { (k, k) } else { (k << 3, k + 8) };
                save[s] = pset[s];
                let v = pset[s].wrapping_mul(scale as i16);
                block[pos] =
                    if !uniform && v != 0 { v.wrapping_add(if v < 0 { -q } else { q } as i16) } else { v };
            }
            if left {
                save[0] = pset[0];
            } else {
                save[8] = pset[8];
            }
        }
        self.st.ac[pl][idx] = save;
    }

    // ---- I frames --------------------------------------------------------------------------------

    pub(super) fn i_frame(&mut self) -> Result<()> {
        let f = self.f;
        self.codingset = match f.y_ac {
            0 if f.pqindex <= 8 => CS_HIGH_RATE_INTRA,
            0 => CS_LOW_MOT_INTRA,
            1 => CS_HIGH_MOT_INTRA,
            _ => CS_MID_RATE_INTRA,
        };
        self.codingset2 = inter_set(f);
        for my in 0..self.mbh {
            for mx in 0..self.mbw {
                let mut cbp = self.codes.mb_i.read(&mut self.r)?;
                let ac_pred = self.r.bit()?;
                for k in 0..6 {
                    let (pl, x, y) = blk(k, mx, my);
                    let mut val = (cbp >> (5 - k)) & 1;
                    if k < 4 {
                        val = self.coded_block_pred(x, y, val as u8) as u32;
                    }
                    cbp |= val << (5 - k);
                    let gw = self.st.gw[pl];
                    self.st.intra[pl][y * gw + x] = true;
                    let mut block = [0i16; 64];
                    let set = if k < 4 { self.codingset } else { self.codingset2 };
                    self.i_block(&mut block, k, mx, my, val != 0, set, ac_pred)?;
                    dsp::inv_8x8(&mut block);
                    let (ox, oy) = blk_origin(k, mx, my);
                    let p = self.cur.plane_mut(pl);
                    dsp::put_clamped(&block, &mut p.data, oy * p.width + ox, p.width);
                }
            }
        }
        if self.seq.loop_filter {
            filter::i_frame(self.cur, self.f.pq);
        }
        Ok(())
    }

    fn coded_block_pred(&mut self, x: usize, y: usize, diff: u8) -> u8 {
        let w = self.st.gw[0];
        let c = &self.st.coded;
        let a = if x > 0 { c[y * w + x - 1] } else { 0 };
        let b = if x > 0 && y > 0 { c[(y - 1) * w + x - 1] } else { 0 };
        let t = if y > 0 { c[(y - 1) * w + x] } else { 0 };
        let pred = if b == t { a } else { t };
        let v = diff ^ pred;
        self.st.coded[y * w + x] = v;
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn i_block(
        &mut self,
        block: &mut [i16; 64],
        n: usize,
        mx: usize,
        my: usize,
        coded: bool,
        set: usize,
        ac_pred: bool,
    ) -> Result<()> {
        let pq = self.f.pq;
        let (pl, x, y) = blk(n, mx, my);
        let w = self.st.gw[pl];
        let idx = y * w + x;
        let dcdiff = self.dc_diff(n >= 4, pq)?;
        let scale = DC_SCALE[pq as usize] as i32;
        let dp = DCPRED[scale as usize];
        let dc = &self.st.dc[pl];
        let a = if y > 0 { dc[idx - w] } else { dp };
        let c = if x > 0 { dc[idx - 1] } else { dp };
        let b = if x > 0 && y > 0 { dc[idx - w - 1] } else { dp };
        let left = (a - b).abs() <= (b - c).abs();
        let dcv = ((if left { c } else { a }) + dcdiff) as i16 as i32;
        self.st.dc[pl][idx] = dcv;
        block[0] = (dcv * scale) as i16;

        let nb = if left { (x > 0).then(|| idx - 1) } else { (y > 0).then(|| idx - w) };
        let pset = nb.map(|i| self.st.ac[pl][i]).unwrap_or([0; 16]);
        if coded {
            let t = if ac_pred { if left { 3 } else { 2 } } else { 1 };
            self.read_intra_ac(block, set, t)?;
        }
        self.finish_intra(block, pl, idx, coded, ac_pred, left, &pset, pq);
        Ok(())
    }

    // ---- P frames --------------------------------------------------------------------------------

    pub(super) fn p_frame(&mut self) -> Result<()> {
        let f = self.f;
        self.codingset = match f.c_ac {
            0 if f.pqindex <= 8 => CS_HIGH_RATE_INTRA,
            0 => CS_LOW_MOT_INTRA,
            1 => CS_HIGH_MOT_INTRA,
            _ => CS_MID_RATE_INTRA,
        };
        self.codingset2 = inter_set(f);
        for my in 0..self.mbh {
            for mx in 0..self.mbw {
                self.p_mb(mx, my)?;
            }
        }
        if self.seq.loop_filter {
            filter::p_frame(self.cur, self.st, self.mbw, self.mbh, self.f.pq);
        }
        Ok(())
    }

    /// Motion vector data: (dx, dy, intra, has coefficients).
    fn mv_data(&mut self) -> Result<(i32, i32, bool, bool)> {
        let f = self.f;
        let mut index = 1 + self.codes.mv_diff[f.mv_table].read(&mut self.r)? as usize;
        let has = index > 36;
        if has {
            index -= 37;
        }
        match index {
            0 => return Ok((0, 0, false, has)),
            35 => {
                let q = f.quarter as u32;
                let x = self.r.bits(f.k_x - 1 + q)? as i32;
                let y = self.r.bits(f.k_y - 1 + q)? as i32;
                return Ok((x, y, false, has));
            }
            36 => return Ok((0, 0, true, has)),
            _ => {}
        }
        let mut comp = |i: usize| -> Result<i32> {
            let mut d = MV_OFFSET[i];
            let n = MV_SIZE[i] - (!f.quarter && i == 5) as u32;
            if n > 0 {
                let val = self.r.bits(n)? as i32;
                let sign = -(val & 1);
                d = (sign ^ ((val >> 1) + d)) - sign;
            }
            Ok(d)
        };
        let x = comp(index % 6)?;
        let y = comp(index / 6)?;
        Ok((x, y, false, has))
    }

    /// Predict and store the vector of luma block `n` (or all four with `one`).
    #[allow(clippy::too_many_arguments)]
    fn pred_mv(
        &mut self,
        mx: usize,
        my: usize,
        n: usize,
        dmx: i32,
        dmy: i32,
        one: bool,
        intra: bool,
    ) -> Result<()> {
        let w = self.st.gw[0];
        let xy = (2 * my + (n >> 1)) * w + 2 * mx + (n & 1);
        let store = |st: &mut State, v: [i32; 2]| {
            st.mv[xy] = v;
            if one {
                st.mv[xy + 1] = v;
                st.mv[xy + w] = v;
                st.mv[xy + w + 1] = v;
            }
        };
        if intra {
            store(self.st, [0, 0]);
            return Ok(());
        }
        let (dmx, dmy) = if self.f.quarter { (dmx, dmy) } else { (dmx * 2, dmy * 2) };
        let a_valid = my > 0 || n == 2 || n == 3;
        let mut b_valid = a_valid;
        let c_valid = mx > 0 || n == 1 || n == 3;
        let last_col = mx == self.mbw - 1;
        let off: isize = if one {
            b_valid &= self.mbw > 1;
            if last_col { -1 } else { 2 }
        } else {
            match n {
                0 => {
                    if mx > 0 {
                        -1
                    } else {
                        1
                    }
                }
                1 => {
                    if last_col {
                        -1
                    } else {
                        1
                    }
                }
                2 => 1,
                _ => -1,
            }
        };
        let mv = &self.st.mv;
        let a = if a_valid { mv[xy - w] } else { [0, 0] };
        let b = if b_valid { mv[((xy - w) as isize + off) as usize] } else { [0, 0] };
        let c = if c_valid { mv[xy - 1] } else { [0, 0] };
        let (mut px, mut py) = if a_valid {
            (a[0], a[1])
        } else if c_valid {
            (c[0], c[1])
        } else if b_valid {
            (b[0], b[1])
        } else {
            (0, 0)
        };
        if a_valid as u32 + b_valid as u32 + c_valid as u32 > 1 {
            px = mid_pred(a[0], b[0], c[0]);
            py = mid_pred(a[1], b[1], c[1]);
        }
        // Pull the predictor back so the referenced block stays near the picture.
        let lim = if one { -60 } else { -28 };
        let qx = ((mx as i32) << 6) + if n == 1 || n == 3 { 32 } else { 0 };
        let qy = ((my as i32) << 6) + if n == 2 || n == 3 { 32 } else { 0 };
        let xmax = ((self.mbw as i32) << 6) - 4;
        let ymax = ((self.mbh as i32) << 6) - 4;
        if qx + px < lim {
            px = lim - qx;
        }
        if qy + py < lim {
            py = lim - qy;
        }
        if qx + px > xmax {
            px = xmax - qx;
        }
        if qy + py > ymax {
            py = ymax - qy;
        }
        // Hybrid prediction: an explicit choice between A and C when the median is far from one of them.
        if a_valid && c_valid {
            let intra = &self.st.intra[0];
            let dist = |o: [i32; 2], o_intra: bool| {
                if o_intra { px.abs() + py.abs() } else { (px - o[0]).abs() + (py - o[1]).abs() }
            };
            let pick = dist(a, intra[xy - w]) > 32 || dist(c, intra[xy - 1]) > 32;
            if pick {
                let p = if self.r.bit()? { a } else { c };
                (px, py) = (p[0], p[1]);
            }
        }
        let (rx, ry) = (self.f.range_x, self.f.range_y);
        let vx = ((px + dmx + rx) & ((rx << 1) - 1)) - rx;
        let vy = ((py + dmy + ry) & ((ry << 1) - 1)) - ry;
        store(self.st, [vx, vy]);
        Ok(())
    }

    /// Chroma vector from a luma vector component.
    fn chroma_component(v: i32) -> i32 {
        (v + ((v & 3) == 3) as i32) >> 1
    }

    fn chroma_mc(&mut self, mx: usize, my: usize, uvmx: i32, uvmy: i32) {
        let cw = (self.mbw * 8) as i32;
        let ch = (self.mbh * 8) as i32;
        let sx = (mx as i32 * 8 + (uvmx >> 2)).clamp(-8, cw);
        let sy = (my as i32 * 8 + (uvmy >> 2)).clamp(-8, ch);
        let (fx, fy) = ((uvmx & 3) << 1, (uvmy & 3) << 1);
        let f: &Frame = self.f;
        let lut = f.lut.as_ref().map(|l| &l.1);
        mc::chroma(&mut self.cur.cb, mx * 8, my * 8, &self.refp.cb, sx, sy, fx, fy, self.rnd, lut);
        mc::chroma(&mut self.cur.cr, mx * 8, my * 8, &self.refp.cr, sx, sy, fx, fy, self.rnd, lut);
    }

    fn mc_1mv(&mut self, mx: usize, my: usize) {
        let w = self.st.gw[0];
        let [vx, vy] = self.st.mv[2 * my * w + 2 * mx];
        let lw = (self.mbw * 16) as i32;
        let lh = (self.mbh * 16) as i32;
        let sx = (mx as i32 * 16 + (vx >> 2)).clamp(-16, lw);
        let sy = (my as i32 * 16 + (vy >> 2)).clamp(-16, lh);
        let f: &Frame = self.f;
        let lut = f.lut.as_ref().map(|l| &l.0);
        mc::luma(
            &mut self.cur.y,
            mx * 16,
            my * 16,
            &self.refp.y,
            sx,
            sy,
            vx & 3,
            vy & 3,
            16,
            self.f.mspel,
            self.rnd,
            lut,
        );
        let (uvmx, uvmy) = (Self::chroma_component(vx), Self::chroma_component(vy));
        self.st.cmv[my * self.mbw + mx] = [uvmx, uvmy];
        self.chroma_mc(mx, my, uvmx, uvmy);
    }

    fn mc_4mv_luma(&mut self, mx: usize, my: usize, n: usize) {
        let w = self.st.gw[0];
        let [vx, vy] = self.st.mv[(2 * my + (n >> 1)) * w + 2 * mx + (n & 1)];
        let lw = (self.mbw * 16) as i32;
        let lh = (self.mbh * 16) as i32;
        let (bx, by) = (mx * 16 + (n & 1) * 8, my * 16 + (n & 2) * 4);
        let sx = (bx as i32 + (vx >> 2)).clamp(-16, lw);
        let sy = (by as i32 + (vy >> 2)).clamp(-16, lh);
        let f: &Frame = self.f;
        let lut = f.lut.as_ref().map(|l| &l.0);
        mc::luma(
            &mut self.cur.y,
            bx,
            by,
            &self.refp.y,
            sx,
            sy,
            vx & 3,
            vy & 3,
            8,
            self.f.mspel,
            self.rnd,
            lut,
        );
    }

    fn mc_4mv_chroma(&mut self, mx: usize, my: usize) {
        let w = self.st.gw[0];
        let base = 2 * my * w + 2 * mx;
        let at = [base, base + 1, base + w, base + w + 1];
        let mut idx = 0usize;
        for (k, &i) in at.iter().enumerate() {
            if !self.st.intra[0][i] {
                idx |= 1 << k;
            }
        }
        let mv = |k: usize| self.st.mv[at[k]];
        let (tx, ty) = match idx.count_ones() {
            4 => (
                median4(mv(0)[0], mv(1)[0], mv(2)[0], mv(3)[0]),
                median4(mv(0)[1], mv(1)[1], mv(2)[1], mv(3)[1]),
            ),
            3 => {
                let (p, q, r) = ((idx > 0xd) as usize, 1 + (idx > 0xb) as usize, 2 + (idx > 0x7) as usize);
                (mid_pred(mv(p)[0], mv(q)[0], mv(r)[0]), mid_pred(mv(p)[1], mv(q)[1], mv(r)[1]))
            }
            2 => {
                const PAIRS: [u8; 16] = [0, 0, 0, 0x01, 0, 0x02, 0x12, 0, 0, 0x03, 0x13, 0, 0x23, 0, 0, 0];
                let (p, q) = ((PAIRS[idx] >> 4) as usize, (PAIRS[idx] & 15) as usize);
                ((mv(p)[0] + mv(q)[0]) / 2, (mv(p)[1] + mv(q)[1]) / 2)
            }
            _ => {
                self.st.cmv[my * self.mbw + mx] = [0, 0];
                return;
            }
        };
        let (uvmx, uvmy) = (Self::chroma_component(tx), Self::chroma_component(ty));
        self.st.cmv[my * self.mbw + mx] = [uvmx, uvmy];
        self.chroma_mc(mx, my, uvmx, uvmy);
    }

    fn p_mb(&mut self, mx: usize, my: usize) -> Result<()> {
        let f = self.f;
        let mbi = my * self.mbw + mx;
        let fourmv = if f.mv_type_raw { self.r.bit()? } else { f.mv_type[mbi] != 0 };
        let skipped = if f.skip_raw { self.r.bit()? } else { f.skip[mbi] != 0 };
        let w = self.st.gw[0];
        let lidx = |n: usize| (2 * my + (n >> 1)) * w + 2 * mx + (n & 1);
        let mut is_intra = [false; 6];
        let mut is_coded = [false; 6];
        let ac_pred;
        let mut ttmb = f.ttfrm;
        if !fourmv {
            if skipped {
                self.pred_mv(mx, my, 0, 0, 0, true, false)?;
                self.mc_1mv(mx, my);
                return Ok(());
            }
            let (dx, dy, intra, has) = self.mv_data()?;
            self.pred_mv(mx, my, 0, dx, dy, true, intra)?;
            let cbp;
            if intra && !has {
                ac_pred = self.r.bit()?;
                cbp = 0;
            } else if has {
                ac_pred = intra && self.r.bit()?;
                cbp = self.codes.cbpcy[f.cbptab].read(&mut self.r)?;
            } else {
                ac_pred = false;
                cbp = 0;
            }
            if !f.ttmbf && !intra && has {
                ttmb = self.codes.ttmb[f.tt_index].read(&mut self.r)? as i32;
            }
            if !intra {
                self.mc_1mv(mx, my);
            }
            for i in 0..6 {
                is_intra[i] = intra;
                is_coded[i] = (cbp >> (5 - i)) & 1 != 0;
            }
            for n in 0..4 {
                self.st.intra[0][lidx(n)] = intra;
            }
            self.st.intra[1][mbi] = intra;
            self.st.intra[2][mbi] = intra;
        } else {
            if skipped {
                for n in 0..4 {
                    self.pred_mv(mx, my, n, 0, 0, false, false)?;
                    self.mc_4mv_luma(mx, my, n);
                }
                self.mc_4mv_chroma(mx, my);
                return Ok(());
            }
            let cbp = self.codes.cbpcy[f.cbptab].read(&mut self.r)?;
            let mut intra_count = 0;
            let mut coded_inter = false;
            for i in 0..6 {
                let val = (cbp >> (5 - i)) & 1 != 0;
                if i < 4 {
                    let (dx, dy, intra, has) = if val { self.mv_data()? } else { (0, 0, false, false) };
                    self.pred_mv(mx, my, i, dx, dy, false, intra)?;
                    if !intra {
                        self.mc_4mv_luma(mx, my, i);
                    }
                    intra_count += intra as u32;
                    is_intra[i] = intra;
                    is_coded[i] = has;
                    self.st.intra[0][lidx(i)] = intra;
                } else {
                    is_intra[i] = intra_count >= 3;
                    is_coded[i] = val;
                    self.st.intra[i - 3][mbi] = is_intra[i];
                }
                if i == 4 {
                    self.mc_4mv_chroma(mx, my);
                }
                coded_inter |= !is_intra[i] && is_coded[i];
            }
            if intra_count == 0 && !coded_inter {
                return Ok(());
            }
            let mut intrapred = false;
            for (i, &intra) in is_intra.iter().enumerate() {
                if intra {
                    let (pl, x, y) = blk(i, mx, my);
                    let gw = self.st.gw[pl];
                    let fl = &self.st.intra[pl];
                    if (y > 0 && fl[(y - 1) * gw + x]) || (x > 0 && fl[y * gw + x - 1]) {
                        intrapred = true;
                        break;
                    }
                }
            }
            ac_pred = intrapred && self.r.bit()?;
            if !f.ttmbf && coded_inter {
                ttmb = self.codes.ttmb[f.tt_index].read(&mut self.r)? as i32;
            }
        }
        let (mut cbp_lf, mut tt_lf, mut intra_lf) = (0u32, 0u32, 0u8);
        let mut first = true;
        for i in 0..6 {
            if is_intra[i] {
                self.intra_mb_block(i, mx, my, is_coded[i], ac_pred)?;
                cbp_lf |= 0xF << (i * 4);
                intra_lf |= 1 << i;
            } else if is_coded[i] {
                let (pat, tt) = self.p_block(i, mx, my, ttmb, first)?;
                cbp_lf |= pat << (i * 4);
                tt_lf |= (tt as u32) << (i * 4);
                if !f.ttmbf && ttmb < 8 {
                    ttmb = -1;
                }
                first = false;
            }
        }
        self.st.cbp[mbi] = cbp_lf;
        self.st.tt[mbi] = tt_lf;
        self.st.intra_bits[mbi] = intra_lf;
        Ok(())
    }

    /// Intra block of a P frame: decode, transform and write (signed, +128).
    fn intra_mb_block(&mut self, n: usize, mx: usize, my: usize, coded: bool, ac_pred: bool) -> Result<()> {
        let q = self.f.pq;
        let (pl, x, y) = blk(n, mx, my);
        let w = self.st.gw[pl];
        let idx = y * w + x;
        let fl = &self.st.intra[pl];
        let a_avail = y > 0 && fl[idx - w];
        let c_avail = x > 0 && fl[idx - 1];
        let set = if n < 4 { self.codingset } else { self.codingset2 };
        let mut block = [0i16; 64];

        let dcdiff = self.dc_diff(n >= 4, q)?;
        let scale = DC_SCALE[q as usize] as i32;
        let dc = &self.st.dc[pl];
        let a = if a_avail { dc[idx - w] } else { 0 };
        let c = if c_avail { dc[idx - 1] } else { 0 };
        let b = if a_avail && c_avail { dc[idx - w - 1] } else { 0 };
        let (pred, mut left) = if c_avail && (!a_avail || (a - b).abs() <= (b - c).abs()) {
            (c, true)
        } else if a_avail {
            (a, false)
        } else {
            (0, true)
        };
        let dcv = (pred + dcdiff) as i16 as i32;
        self.st.dc[pl][idx] = dcv;
        block[0] = (dcv * scale) as i16;
        if !a_avail {
            left = true;
        }
        if !c_avail {
            left = false;
        }
        let use_pred = ac_pred && (a_avail || c_avail);
        let pset = if use_pred { self.st.ac[pl][if left { idx - 1 } else { idx - w }] } else { [0; 16] };
        if coded {
            self.read_intra_ac(&mut block, set, 0)?;
        }
        self.finish_intra(&mut block, pl, idx, coded, use_pred, left, &pset, q);
        dsp::inv_8x8(&mut block);
        let (ox, oy) = blk_origin(n, mx, my);
        let p = self.cur.plane_mut(pl);
        dsp::put_signed_clamped(&block, &mut p.data, oy * p.width + ox, p.width);
        Ok(())
    }

    /// Decode one inter (sub)block's coefficients along `scan` (positions up to `max`), dequantized; returns the
    /// position after the last one.
    fn inter_coeffs(
        &mut self,
        block: &mut [i16; 64],
        scan: &dyn Fn(usize) -> usize,
        max: usize,
    ) -> Result<usize> {
        let f = self.f;
        let (q, scale, uniform) = (f.pq, 2 * f.pq + f.halfpq, f.pquantizer);
        let mut i = 0usize;
        loop {
            let (last, run, value) = self.ac_symbol(self.codingset2)?;
            i += run;
            if i > max {
                break;
            }
            block[scan(i)] = requant(value as i16, scale, q, uniform);
            i += 1;
            if last {
                break;
            }
        }
        Ok(i)
    }

    /// Inter block residual: decode and add to the prediction. Returns (coded 4x4 pattern, transform type).
    fn p_block(&mut self, n: usize, mx: usize, my: usize, ttmb: i32, first: bool) -> Result<(u32, i32)> {
        let f = self.f;
        let mut ttblk = ttmb & 7;
        if ttmb == -1 {
            let s = self.codes.ttblk[f.tt_index].read(&mut self.r)? as usize;
            ttblk = TTBLK_TO_TT[f.tt_index * 8 + s] as i32;
        }
        let mut subblkpat: i32 = 0;
        if ttblk == TT_4X4 {
            subblkpat = !(self.codes.subblkpat[f.tt_index].read(&mut self.r)? as i32 + 1);
        }
        if ttblk != TT_8X8 && ttblk != TT_4X4 && (f.ttmbf || (ttmb != -1 && (ttmb & 8) != 0 && !first)) {
            subblkpat = decode012(&mut self.r)? as i32;
            if subblkpat != 0 {
                subblkpat ^= 3;
            }
            if ttblk == TT_8X4_TOP || ttblk == TT_8X4_BOTTOM {
                ttblk = TT_8X4;
            }
            if ttblk == TT_4X8_RIGHT || ttblk == TT_4X8_LEFT {
                ttblk = TT_4X8;
            }
        }
        if ttblk == TT_8X4_TOP || ttblk == TT_8X4_BOTTOM {
            subblkpat = 2 - (ttblk == TT_8X4_TOP) as i32;
            ttblk = TT_8X4;
        }
        if ttblk == TT_4X8_RIGHT || ttblk == TT_4X8_LEFT {
            subblkpat = 2 - (ttblk == TT_4X8_LEFT) as i32;
            ttblk = TT_4X8;
        }
        let (pl, _, _) = blk(n, mx, my);
        let (ox, oy) = blk_origin(n, mx, my);
        let stride = self.cur.plane(pl).width;
        let base = oy * stride + ox;

        let pat;
        match ttblk {
            TT_8X8 => {
                pat = 0xF;
                let mut block = [0i16; 64];
                let i = self.inter_coeffs(&mut block, &|i| zz(0, i), 63)?;
                let p = self.cur.plane_mut(pl);
                if i == 1 {
                    dsp::inv_8x8_dc_add(&mut p.data, base, stride, &block);
                } else {
                    dsp::inv_8x8(&mut block);
                    dsp::add_clamped(&block, &mut p.data, base, stride);
                }
            }
            TT_4X4 => {
                pat = (!subblkpat & 0xF) as u32;
                for j in 0..4 {
                    if subblkpat & (1 << (3 - j)) != 0 {
                        continue;
                    }
                    let mut block = [0i16; 64];
                    let i = self.inter_coeffs(&mut block, &|i| ZZ_4X4[i] as usize, 15)?;
                    let at = base + (j & 1) * 4 + (j & 2) * 2 * stride;
                    let p = self.cur.plane_mut(pl);
                    if i == 1 {
                        dsp::inv_4x4_dc_add(&mut p.data, at, stride, &block);
                    } else {
                        dsp::inv_4x4_add(&mut p.data, at, stride, &mut block);
                    }
                }
            }
            TT_8X4 => {
                pat = (!((subblkpat & 2) * 6 + (subblkpat & 1) * 3) & 0xF) as u32;
                for j in 0..2 {
                    if subblkpat & (1 << (1 - j)) != 0 {
                        continue;
                    }
                    let mut block = [0i16; 64];
                    let i = self.inter_coeffs(&mut block, &|i| SCAN_8X4[i] as usize, 31)?;
                    let at = base + j * 4 * stride;
                    let p = self.cur.plane_mut(pl);
                    if i == 1 {
                        dsp::inv_8x4_dc_add(&mut p.data, at, stride, &block);
                    } else {
                        dsp::inv_8x4_add(&mut p.data, at, stride, &mut block);
                    }
                }
            }
            TT_4X8 => {
                pat = (!(subblkpat * 5) & 0xF) as u32;
                for j in 0..2 {
                    if subblkpat & (1 << (1 - j)) != 0 {
                        continue;
                    }
                    let mut block = [0i16; 64];
                    let i = self.inter_coeffs(&mut block, &|i| SCAN_4X8[i] as usize, 31)?;
                    let at = base + j * 4;
                    let p = self.cur.plane_mut(pl);
                    if i == 1 {
                        dsp::inv_4x8_dc_add(&mut p.data, at, stride, &block);
                    } else {
                        dsp::inv_4x8_add(&mut p.data, at, stride, &mut block);
                    }
                }
            }
            t => anyhow::bail!("transform type {t}"),
        }
        Ok((pat, ttblk))
    }
}

/// The inter coefficient set chosen by the frame's chroma/inter table index.
fn inter_set(f: &Frame) -> usize {
    match f.c_ac {
        0 if f.pqindex <= 8 => CS_HIGH_RATE_INTER,
        0 => CS_LOW_MOT_INTER,
        1 => CS_HIGH_MOT_INTER,
        _ => CS_MID_RATE_INTER,
    }
}
