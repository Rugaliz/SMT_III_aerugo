//! Inverse MDCT: N coefficients to 2N windowed-ready samples.
//!
//! With the standard definition `y[n] = Σ_k X[k] cos(π/N (n + 1/2 + N/2)(k + 1/2))`, the output used by the
//! decoder is, for `i < N/2`: `out[N/2 + i] = s·y[N/2 - 1 - i]` and `out[N + i] = -s·y[N + i]`, then mirrored:
//! `out[i] = -out[N - 1 - i]` and `out[2N - 1 - i] = out[N + i]`.
//! `y` is computed through a 2N-point complex FFT of the twiddled, zero-padded coefficients.

use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, Default)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    fn mul(self, o: C) -> C {
        C { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
    fn cis(a: f64) -> C {
        C { re: a.cos(), im: a.sin() }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Imdct {
    n: usize,
    scale: f64,
    pre: Vec<C>,
    post: Vec<C>,
    roots: Vec<C>,
    rev: Vec<usize>,
}

impl Imdct {
    pub fn new(n: usize, scale: f64) -> Self {
        let m = 2 * n;
        let n0 = 0.5 + n as f64 / 2.0;
        let pre = (0..n).map(|k| C::cis(PI * n0 * k as f64 / n as f64)).collect();
        let post = (0..m).map(|i| C::cis(PI * (i as f64 + n0) / (2.0 * n as f64))).collect();
        let roots = (0..m / 2).map(|k| C::cis(2.0 * PI * k as f64 / m as f64)).collect();
        let bits = m.trailing_zeros();
        let rev = (0..m).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        Self { n, scale, pre, post, roots, rev }
    }

    /// The standard inverse MDCT `y` (2N values).
    fn standard(&self, x: &[f32]) -> Vec<f64> {
        let m = 2 * self.n;
        let mut a = vec![C::default(); m];
        for k in 0..self.n {
            a[self.rev[k]] = C { re: x[k] as f64, im: 0.0 }.mul(self.pre[k]);
        }
        // Iterative radix-2 FFT with positive exponent (inverse direction, unscaled).
        let mut len = 2;
        while len <= m {
            let step = m / len;
            for start in (0..m).step_by(len) {
                for j in 0..len / 2 {
                    let w = self.roots[j * step];
                    let u = a[start + j];
                    let v = a[start + j + len / 2].mul(w);
                    a[start + j] = C { re: u.re + v.re, im: u.im + v.im };
                    a[start + j + len / 2] = C { re: u.re - v.re, im: u.im - v.im };
                }
            }
            len *= 2;
        }
        (0..m).map(|i| a[i].mul(self.post[i]).re).collect()
    }

    /// The decoder's full output (2N samples).
    pub fn full(&self, x: &[f32], out: &mut [f32]) {
        let n = self.n;
        let y = self.standard(x);
        let s = self.scale;
        for i in 0..n / 2 {
            out[n / 2 + i] = (s * y[n / 2 - 1 - i]) as f32;
            out[n + i] = (-s * y[n + i]) as f32;
        }
        for i in 0..n / 2 {
            out[i] = -out[n - 1 - i];
            out[2 * n - 1 - i] = out[n + i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference definition: half transform with explicit cosine sums, then mirroring.
    fn naive_full(x: &[f32], scale: f64) -> Vec<f32> {
        let len2 = x.len();
        let len = len2 / 2;
        let phase = PI / (4.0 * len2 as f64);
        let mut h = vec![0f64; len2];
        for i in 0..len {
            let (mut d, mut u) = (0f64, 0f64);
            let id = phase * (4 * len - 2 * i - 1) as f64;
            let iu = phase * (3 * len2 + 2 * i + 1) as f64;
            for (j, &v) in x.iter().enumerate() {
                let a = (2 * j + 1) as f64;
                d += (a * id).cos() * v as f64;
                u += (a * iu).cos() * v as f64;
            }
            h[i] = d * scale;
            h[i + len] = -u * scale;
        }
        let n = len2;
        let mut out = vec![0f32; 2 * n];
        for i in 0..n {
            out[n / 2 + i] = h[i] as f32;
        }
        for i in 0..n / 2 {
            out[i] = -out[n - 1 - i];
            out[2 * n - 1 - i] = out[n + i];
        }
        out
    }

    #[test]
    fn matches_the_reference_definition() {
        for n in [128usize, 512, 2048] {
            let x: Vec<f32> = (0..n).map(|i| ((i * 7919 % 1000) as f32 - 500.0) / 37.0).collect();
            let want = naive_full(&x, 1.0 / 32768.0);
            let mut got = vec![0f32; 2 * n];
            Imdct::new(n, 1.0 / 32768.0).full(&x, &mut got);
            let err = want.iter().zip(&got).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            let peak = want.iter().map(|v| v.abs()).fold(0f32, f32::max);
            assert!(err <= peak * 1e-5, "n {n}: max error {err} of peak {peak}");
        }
    }
}
