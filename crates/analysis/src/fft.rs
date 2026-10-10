//! Radix-2 fast Fourier transform (iterative Cooley–Tukey), as the ToolPak's Fourier Analysis
//! uses it: the forward transform is unscaled, X[k] = Σ x[n]·e^(−2πikn/N); the inverse divides by
//! N, so inverse(forward(x)) = x.

use crate::{Error, Result};

/// Largest transform (the ToolPak's limit).
pub const MAX_LEN: usize = 4096;

/// A complex number (re, im).
pub type Complex = (f64, f64);

/// Transforms `x` (length a power of two, 1..=4096).
pub fn fft(x: &[Complex], inverse: bool) -> Result<Vec<Complex>> {
    let n = x.len();
    if n == 0 || !n.is_power_of_two() {
        return Err(Error::Input("the number of input values must be a power of 2".into()));
    }
    if n > MAX_LEN {
        return Err(Error::TooLarge(format!("at most {MAX_LEN} input values")));
    }
    let bits = n.trailing_zeros();
    // Bit-reversed copy.
    let mut a: Vec<Complex> = vec![(0.0, 0.0); n];
    for (i, v) in x.iter().enumerate() {
        let j = if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) };
        if let Some(slot) = a.get_mut(j) {
            *slot = *v;
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * 2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                // Twiddle computed directly (not by repeated multiplication) for accuracy.
                let (wr, wi) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (i, j) = (start + k, start + k + len / 2);
                let (ur, ui) = a.get(i).copied().unwrap_or((0.0, 0.0));
                let (xr, xi) = a.get(j).copied().unwrap_or((0.0, 0.0));
                let (vr, vi) = (xr * wr - xi * wi, xr * wi + xi * wr);
                if let Some(s) = a.get_mut(i) {
                    *s = (ur + vr, ui + vi);
                }
                if let Some(s) = a.get_mut(j) {
                    *s = (ur - vr, ui - vi);
                }
            }
        }
        len <<= 1;
    }
    if inverse {
        let nf = n as f64;
        a.iter_mut().for_each(|v| *v = (v.0 / nf, v.1 / nf));
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Complex, b: Complex) {
        assert!((a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12, "{a:?} vs {b:?}");
    }

    #[test]
    fn transforms_and_round_trips() {
        let x: Vec<Complex> = [1.0, 2.0, 3.0, 4.0].iter().map(|v| (*v, 0.0)).collect();
        let f = fft(&x, false).unwrap();
        for (got, want) in f.iter().zip([(10.0, 0.0), (-2.0, 2.0), (-2.0, 0.0), (-2.0, -2.0)]) {
            close(*got, want);
        }
        let back = fft(&f, true).unwrap();
        for (got, want) in back.iter().zip(&x) {
            close(*got, *want);
        }
        close(fft(&[(5.0, -1.0)], false).unwrap()[0], (5.0, -1.0));
        // A long random-ish signal survives the round trip.
        let y: Vec<Complex> = (0..1024).map(|i| (((i * 7919) % 101) as f64, ((i * 31) % 17) as f64)).collect();
        let r = fft(&fft(&y, false).unwrap(), true).unwrap();
        for (got, want) in r.iter().zip(&y) {
            assert!((got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9);
        }
    }

    #[test]
    fn rejects_bad_lengths() {
        assert!(fft(&[], false).is_err());
        assert!(fft(&[(1.0, 0.0); 3], false).is_err());
        assert!(fft(&vec![(1.0, 0.0); MAX_LEN * 2], false).is_err());
    }
}
