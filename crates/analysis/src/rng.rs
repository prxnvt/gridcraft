//! Seeded random numbers: the xoshiro256** generator (Blackman and Vigna, public domain
//! algorithm) seeded through SplitMix64, and the distributions the ToolPak's Random Number
//! Generation and Sampling tools draw from.
//!
//! The same seed always gives the same numbers in GridCraft. They are not Excel's numbers:
//! Excel's generator is not documented, and we don't reverse-engineer it.

use gridcraft_functions::dist;

/// A xoshiro256** generator.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut x = seed;
        Rng { s: [splitmix(&mut x), splitmix(&mut x), splitmix(&mut x), splitmix(&mut x)] }
    }

    /// Seeded from the clock (for "no seed given").
    pub fn from_time() -> Rng {
        #[cfg(not(target_arch = "wasm32"))]
        #[allow(clippy::disallowed_methods)] // never on wasm (see above)
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x1234_5678);
        #[cfg(target_arch = "wasm32")]
        let t = 0x1234_5678u64;
        Rng::new(t)
    }

    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.s;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s1 << 17;
        let mut s = [s0, s1, s2 ^ s0, s3 ^ s1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        self.s = s;
        result
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in (0, 1) (never 0, for logarithms and inverse CDFs).
    fn open01(&mut self) -> f64 {
        loop {
            let u = self.uniform();
            if u > 0.0 {
                return u;
            }
        }
    }

    /// Uniform integer in [0, n) (n ≥ 1).
    pub fn below(&mut self, n: u64) -> u64 {
        if n <= 1 {
            return 0;
        }
        // Rejection to avoid modulo bias.
        let zone = u64::MAX - u64::MAX % n;
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % n;
            }
        }
    }

    /// Uniform in [a, b].
    pub fn uniform_between(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.uniform()
    }

    /// Normal with the given mean and standard deviation (inverse-CDF method).
    pub fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        mean + sd * dist::norm_inv(self.open01())
    }

    /// 1 with probability p, else 0.
    pub fn bernoulli(&mut self, p: f64) -> f64 {
        if self.uniform() < p { 1.0 } else { 0.0 }
    }

    /// Picks a value with the given probabilities (which sum to 1).
    pub fn discrete(&mut self, values: &[f64], probs: &[f64]) -> f64 {
        let u = self.uniform();
        let mut acc = 0.0;
        for (v, p) in values.iter().zip(probs) {
            acc += p;
            if u < acc {
                return *v;
            }
        }
        values.last().copied().unwrap_or(0.0)
    }
}

/// Most outcomes tabulated for a binomial or Poisson sampler.
pub const MAX_TABLE: usize = 2_000_000;

/// A discrete distribution on 0, 1, 2, … sampled by inverting its tabulated CDF. Build it once
/// per tool run, then draw as many numbers as needed.
#[derive(Clone, Debug)]
pub struct Table {
    cdf: Vec<f64>,
}

impl Table {
    fn from_pmf(n: usize, pmf: impl Fn(f64) -> f64) -> Table {
        let mut cdf = Vec::with_capacity(n.min(MAX_TABLE) + 1);
        let mut acc = 0.0;
        for k in 0..=n.min(MAX_TABLE) {
            acc += pmf(k as f64);
            cdf.push(acc);
            if acc >= 1.0 - 1e-16 && k > 0 {
                break;
            }
        }
        Table { cdf }
    }
    /// Successes in `n` trials with success probability `p`.
    pub fn binomial(p: f64, n: u64) -> Table {
        let nf = n as f64;
        Table::from_pmf(usize::try_from(n).unwrap_or(MAX_TABLE), |k| dist::binom_pmf(k, nf, p))
    }
    /// Poisson with mean `lambda`.
    pub fn poisson(lambda: f64) -> Table {
        let lambda = lambda.max(0.0);
        let span = (lambda + 40.0 * lambda.sqrt() + 100.0).min(MAX_TABLE as f64) as usize;
        Table::from_pmf(span, |k| dist::poisson_pmf(k, lambda))
    }
    /// One draw.
    pub fn sample(&self, r: &mut Rng) -> f64 {
        let total = self.cdf.last().copied().unwrap_or(1.0);
        let u = r.uniform() * total;
        self.cdf.partition_point(|c| *c <= u).min(self.cdf.len().saturating_sub(1)) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_numbers() {
        let a: Vec<u64> = {
            let mut r = Rng::new(42);
            (0..5).map(|_| r.next_u64()).collect()
        };
        let mut r = Rng::new(42);
        assert_eq!(a, (0..5).map(|_| r.next_u64()).collect::<Vec<_>>());
        assert_ne!(a[0], Rng::new(43).next_u64());
    }

    fn mean_var(v: &[f64]) -> (f64, f64) {
        let m = v.iter().sum::<f64>() / v.len() as f64;
        (m, v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64)
    }

    #[test]
    fn distributions_have_the_right_moments() {
        let mut r = Rng::new(7);
        let n = 20_000;
        let u: Vec<f64> = (0..n).map(|_| r.uniform_between(2.0, 4.0)).collect();
        assert!(u.iter().all(|x| (2.0..=4.0).contains(x)));
        let (m, v) = mean_var(&u);
        assert!((m - 3.0).abs() < 0.03 && (v - 1.0 / 3.0).abs() < 0.02, "{m} {v}");
        let z: Vec<f64> = (0..n).map(|_| r.normal(10.0, 2.0)).collect();
        let (m, v) = mean_var(&z);
        assert!((m - 10.0).abs() < 0.05 && (v - 4.0).abs() < 0.15, "{m} {v}");
        let t = Table::binomial(0.3, 20);
        let b: Vec<f64> = (0..n).map(|_| t.sample(&mut r)).collect();
        let (m, v) = mean_var(&b);
        assert!((m - 6.0).abs() < 0.08 && (v - 4.2).abs() < 0.2, "{m} {v}");
        let t = Table::binomial(0.5, 100_000);
        let big: Vec<f64> = (0..2000).map(|_| t.sample(&mut r)).collect();
        assert!((mean_var(&big).0 - 50_000.0).abs() < 20.0);
        assert_eq!(Table::binomial(0.0, 10).sample(&mut r), 0.0);
        assert_eq!(Table::binomial(1.0, 10).sample(&mut r), 10.0);
        for lambda in [3.5, 80.0] {
            let t = Table::poisson(lambda);
            let p: Vec<f64> = (0..n).map(|_| t.sample(&mut r)).collect();
            let (m, v) = mean_var(&p);
            assert!((m - lambda).abs() < lambda * 0.03 && (v - lambda).abs() < lambda * 0.08, "{lambda}: {m} {v}");
        }
        let d: Vec<f64> = (0..n).map(|_| r.discrete(&[1.0, 2.0], &[0.25, 0.75])).collect();
        assert!((d.iter().filter(|x| **x == 1.0).count() as f64 / n as f64 - 0.25).abs() < 0.02);
        assert_eq!(r.bernoulli(0.0), 0.0);
        assert_eq!(r.bernoulli(1.0), 1.0);
        assert!((0..1000).all(|_| r.below(7) < 7));
        assert_eq!(r.below(0), 0);
    }
}
