//! Minimal affine math for laying out UI objects.

/// A 3D affine transform: `p' = m * p + t` (column vectors).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub m: [[f32; 3]; 3],
    pub t: [f32; 3],
}

impl Affine {
    pub const IDENTITY: Affine =
        Affine { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], t: [0.0; 3] };

    /// Translation, then rotation (unit quaternion x, y, z, w), then scale: `T * R * S`.
    pub fn trs(t: [f32; 3], q: [f32; 4], s: [f32; 3]) -> Self {
        let [x, y, z, w] = q;
        let r = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
        ];
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = r[i][j] * s[j];
            }
        }
        Affine { m, t }
    }

    pub fn mul(&self, o: &Affine) -> Affine {
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| self.m[i][k] * o.m[k][j]).sum();
            }
        }
        Affine { m, t: self.apply(o.t) }
    }

    /// The inverse transform (identity for a singular one).
    pub fn inverse(&self) -> Affine {
        let m = self.m;
        let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
        let det = m[0][0] * c(1, 1, 2, 2) - m[0][1] * c(1, 0, 2, 2) + m[0][2] * c(1, 0, 2, 1);
        if det.abs() < 1e-12 {
            return Affine::IDENTITY;
        }
        let im = [
            [c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
            [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
            [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det],
        ];
        let t = [0, 1, 2].map(|i| -(im[i][0] * self.t[0] + im[i][1] * self.t[1] + im[i][2] * self.t[2]));
        Affine { m: im, t }
    }

    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let mut out = self.t;
        for (i, v) in out.iter_mut().enumerate() {
            *v += (0..3).map(|k| self.m[i][k] * p[k]).sum::<f32>();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trs_composes_translation_rotation_scale() {
        // 90 degrees about z: (x, y) -> (-y, x).
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let a = Affine::trs([10.0, 0.0, 0.0], [0.0, 0.0, h, h], [2.0, 1.0, 1.0]);
        let p = a.apply([1.0, 0.0, 0.0]);
        assert!((p[0] - 10.0).abs() < 1e-5 && (p[1] - 2.0).abs() < 1e-5);
        let b = Affine::trs([1.0, 2.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
        assert_eq!(b.mul(&Affine::IDENTITY), b);
    }
}
