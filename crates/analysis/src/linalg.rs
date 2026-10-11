//! Dense linear algebra: a row-major matrix and LU factorisation with partial pivoting
//! (solve, inverse, determinant). Sizes here are small (≤ a few hundred), so dense is right.

use crate::{Error, Result};

/// Largest dimension accepted (Solver has at most a few hundred variables and constraints).
pub const MAX_DIM: usize = 4096;

/// A dense row-major matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl Matrix {
    pub fn zeros(rows: usize, cols: usize) -> Result<Matrix> {
        if rows > MAX_DIM || cols > MAX_DIM {
            return Err(Error::TooLarge(format!("a {rows}×{cols} matrix is too large")));
        }
        Ok(Matrix { rows, cols, data: vec![0.0; rows * cols] })
    }
    pub fn identity(n: usize) -> Result<Matrix> {
        let mut m = Matrix::zeros(n, n)?;
        for i in 0..n {
            m.set(i, i, 1.0);
        }
        Ok(m)
    }
    /// From rows (all the same length).
    pub fn from_rows(rows: &[Vec<f64>]) -> Result<Matrix> {
        let cols = rows.first().map_or(0, Vec::len);
        if rows.iter().any(|r| r.len() != cols) {
            return Err(Error::Input("matrix rows have different lengths".into()));
        }
        let mut m = Matrix::zeros(rows.len(), cols)?;
        for (i, r) in rows.iter().enumerate() {
            for (j, v) in r.iter().enumerate() {
                m.set(i, j, *v);
            }
        }
        Ok(m)
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    /// Element (i, j); 0 outside the matrix.
    pub fn get(&self, i: usize, j: usize) -> f64 {
        if i < self.rows && j < self.cols { self.data.get(i * self.cols + j).copied().unwrap_or(0.0) } else { 0.0 }
    }
    /// Sets element (i, j); ignored outside the matrix.
    pub fn set(&mut self, i: usize, j: usize, v: f64) {
        if i < self.rows
            && j < self.cols
            && let Some(x) = self.data.get_mut(i * self.cols + j)
        {
            *x = v;
        }
    }
    pub fn row(&self, i: usize) -> Vec<f64> {
        (0..self.cols).map(|j| self.get(i, j)).collect()
    }
    pub fn to_rows(&self) -> Vec<Vec<f64>> {
        (0..self.rows).map(|i| self.row(i)).collect()
    }
    pub fn transpose(&self) -> Matrix {
        let mut t = Matrix { rows: self.cols, cols: self.rows, data: vec![0.0; self.data.len()] };
        for i in 0..self.rows {
            for j in 0..self.cols {
                t.set(j, i, self.get(i, j));
            }
        }
        t
    }
    pub fn mul(&self, b: &Matrix) -> Result<Matrix> {
        if self.cols != b.rows {
            return Err(Error::Input("matrix sizes don't match".into()));
        }
        let mut m = Matrix::zeros(self.rows, b.cols)?;
        for i in 0..self.rows {
            for k in 0..self.cols {
                let a = self.get(i, k);
                if a == 0.0 {
                    continue;
                }
                for j in 0..b.cols {
                    m.set(i, j, m.get(i, j) + a * b.get(k, j));
                }
            }
        }
        Ok(m)
    }
    /// Matrix × vector.
    pub fn mul_vec(&self, x: &[f64]) -> Vec<f64> {
        (0..self.rows).map(|i| (0..self.cols).map(|j| self.get(i, j) * x.get(j).copied().unwrap_or(0.0)).sum()).collect()
    }
}

/// An LU factorisation P·A = L·U of a square matrix.
#[derive(Clone, Debug)]
pub struct Lu {
    lu: Matrix,
    perm: Vec<usize>,
    sign: f64,
}

/// Pivots smaller than this (relative to the largest entry of the matrix) count as zero.
const SINGULAR: f64 = 1e-13;

impl Lu {
    /// Factorises `a`; a singular matrix is an error.
    pub fn new(a: &Matrix) -> Result<Lu> {
        let n = a.rows();
        if a.cols() != n {
            return Err(Error::Input("the matrix must be square".into()));
        }
        let mut lu = a.clone();
        let mut perm: Vec<usize> = (0..n).collect();
        let mut sign = 1.0;
        let scale = a.data.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(f64::MIN_POSITIVE);
        for k in 0..n {
            // Partial pivoting: largest entry in column k at or below the diagonal.
            let (p, big) = (k..n).map(|i| (i, lu.get(i, k).abs())).fold((k, -1.0), |best, x| if x.1 > best.1 { x } else { best });
            if big.is_nan() || big <= SINGULAR * scale {
                return Err(Error::Numeric("the matrix is singular".into()));
            }
            if p != k {
                for j in 0..n {
                    let (x, y) = (lu.get(k, j), lu.get(p, j));
                    lu.set(k, j, y);
                    lu.set(p, j, x);
                }
                perm.swap(k, p);
                sign = -sign;
            }
            let piv = lu.get(k, k);
            for i in k + 1..n {
                let f = lu.get(i, k) / piv;
                lu.set(i, k, f);
                if f != 0.0 {
                    for j in k + 1..n {
                        lu.set(i, j, lu.get(i, j) - f * lu.get(k, j));
                    }
                }
            }
        }
        Ok(Lu { lu, perm, sign })
    }

    /// Solves A·x = b.
    pub fn solve(&self, b: &[f64]) -> Result<Vec<f64>> {
        let n = self.lu.rows();
        if b.len() != n {
            return Err(Error::Input("the right-hand side has the wrong length".into()));
        }
        let mut x: Vec<f64> = self.perm.iter().map(|&p| b.get(p).copied().unwrap_or(0.0)).collect();
        for i in 0..n {
            let s: f64 = (0..i).map(|j| self.lu.get(i, j) * x.get(j).copied().unwrap_or(0.0)).sum();
            if let Some(v) = x.get_mut(i) {
                *v -= s;
            }
        }
        for i in (0..n).rev() {
            let s: f64 = (i + 1..n).map(|j| self.lu.get(i, j) * x.get(j).copied().unwrap_or(0.0)).sum();
            let d = self.lu.get(i, i);
            if let Some(v) = x.get_mut(i) {
                *v = (*v - s) / d;
            }
        }
        if x.iter().all(|v| v.is_finite()) { Ok(x) } else { Err(Error::Numeric("the system has no finite solution".into())) }
    }

    pub fn det(&self) -> f64 {
        (0..self.lu.rows()).fold(self.sign, |d, i| d * self.lu.get(i, i))
    }

    pub fn inverse(&self) -> Result<Matrix> {
        let n = self.lu.rows();
        let mut inv = Matrix::zeros(n, n)?;
        let mut e = vec![0.0; n];
        for j in 0..n {
            e.iter_mut().for_each(|v| *v = 0.0);
            if let Some(v) = e.get_mut(j) {
                *v = 1.0;
            }
            let col = self.solve(&e)?;
            for (i, v) in col.iter().enumerate() {
                inv.set(i, j, *v);
            }
        }
        Ok(inv)
    }
}

/// Solves A·x = b.
pub fn solve(a: &Matrix, b: &[f64]) -> Result<Vec<f64>> {
    Lu::new(a)?.solve(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(rows: &[&[f64]]) -> Matrix {
        Matrix::from_rows(&rows.iter().map(|r| r.to_vec()).collect::<Vec<_>>()).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn solves_inverts_and_takes_determinants() {
        let a = m(&[&[2.0, 1.0, 1.0], &[4.0, -6.0, 0.0], &[-2.0, 7.0, 2.0]]);
        let x = solve(&a, &[5.0, -2.0, 9.0]).unwrap_or_else(|e| panic!("{e}"));
        for (v, want) in x.iter().zip([1.0, 1.0, 2.0]) {
            assert!((v - want).abs() < 1e-12, "{x:?}");
        }
        let lu = Lu::new(&a).unwrap_or_else(|e| panic!("{e}"));
        assert!((lu.det() - -16.0).abs() < 1e-12);
        let id = a.mul(&lu.inverse().unwrap_or_else(|e| panic!("{e}"))).unwrap_or_else(|e| panic!("{e}"));
        for i in 0..3 {
            for j in 0..3 {
                assert!((id.get(i, j) - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
        assert_eq!(a.transpose().get(0, 1), 4.0);
        assert_eq!(a.mul_vec(&[1.0, 1.0, 2.0]), vec![5.0, -2.0, 9.0]);
    }

    #[test]
    fn rejects_singular_and_malformed_input() {
        assert!(Lu::new(&m(&[&[1.0, 2.0], &[2.0, 4.0]])).is_err());
        assert!(Lu::new(&m(&[&[1.0, 2.0, 3.0]])).is_err());
        assert!(Matrix::from_rows(&[vec![1.0], vec![1.0, 2.0]]).is_err());
        assert!(Matrix::zeros(MAX_DIM + 1, 1).is_err());
        assert!(solve(&m(&[&[1.0]]), &[1.0, 2.0]).is_err());
        assert_eq!(m(&[&[1.0]]).get(5, 5), 0.0);
        // An empty system is trivially solved.
        assert_eq!(solve(&Matrix::zeros(0, 0).unwrap_or_else(|e| panic!("{e}")), &[]).ok(), Some(vec![]));
    }
}
