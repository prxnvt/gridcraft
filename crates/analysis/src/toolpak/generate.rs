//! Random Number Generation and Sampling.
//!
//! Numbers come from the seeded generator in [`crate::rng`]: the same seed always gives the
//! same numbers in GridCraft (not the same numbers as Excel, whose generator isn't public).

use super::{Look, Out};
use crate::rng::{Rng, Table};
use crate::{Error, Result, input};

/// Most numbers one run may generate.
pub const MAX_NUMBERS: usize = 1_000_000;

/// A distribution with its parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum Distribution {
    Uniform {
        from: f64,
        to: f64,
    },
    Normal {
        mean: f64,
        sd: f64,
    },
    Bernoulli {
        p: f64,
    },
    Binomial {
        p: f64,
        trials: u64,
    },
    Poisson {
        lambda: f64,
    },
    /// from..=to in steps, each number repeated, the whole sequence repeated.
    Patterned {
        from: f64,
        to: f64,
        step: f64,
        repeat_each: usize,
        repeat_sequence: usize,
    },
    /// (value, probability) pairs; probabilities sum to 1.
    Discrete {
        values: Vec<f64>,
        probs: Vec<f64>,
    },
}

fn check_p(p: f64) -> Result<()> {
    if (0.0..=1.0).contains(&p) { Ok(()) } else { Err(input("The p value must be between 0 and 1.")) }
}

/// The patterned sequence.
fn pattern(from: f64, to: f64, step: f64, each: usize, times: usize) -> Result<Vec<f64>> {
    if !(step > 0.0 && step.is_finite()) || !from.is_finite() || !to.is_finite() {
        return Err(input("The step must be a positive number."));
    }
    if to < from {
        return Err(input("The pattern must run from a smaller number to a larger one."));
    }
    let steps = ((to - from) / step + 1e-9).floor();
    if steps > MAX_NUMBERS as f64 {
        return Err(Error::TooLarge("The pattern is too long.".into()));
    }
    let base: Vec<f64> = (0..=steps as usize).map(|k| from + step * k as f64).collect();
    let len = base.len().saturating_mul(each.max(1)).saturating_mul(times.max(1));
    if len > MAX_NUMBERS {
        return Err(Error::TooLarge("The pattern is too long.".into()));
    }
    let mut out = Vec::with_capacity(len);
    for _ in 0..times.max(1) {
        for v in &base {
            out.extend(std::iter::repeat_n(*v, each.max(1)));
        }
    }
    Ok(out)
}

/// `count` numbers in each of `variables` columns (Patterned ignores `count`: the pattern sets
/// the length).
pub fn random_numbers(variables: usize, count: usize, d: &Distribution, seed: Option<u64>) -> Result<Out> {
    if variables == 0 {
        return Err(input("The number of variables must be at least 1."));
    }
    let mut rng = match seed {
        Some(s) => Rng::new(s),
        None => Rng::from_time(),
    };
    let columns: Vec<Vec<f64>> = match d {
        Distribution::Patterned { from, to, step, repeat_each, repeat_sequence } => {
            let p = pattern(*from, *to, *step, *repeat_each, *repeat_sequence)?;
            if p.len().saturating_mul(variables) > MAX_NUMBERS {
                return Err(Error::TooLarge(format!("At most {MAX_NUMBERS} numbers at a time.")));
            }
            vec![p; variables]
        }
        _ => {
            if count == 0 {
                return Err(input("The number of random numbers must be at least 1."));
            }
            if count.saturating_mul(variables) > MAX_NUMBERS {
                return Err(Error::TooLarge(format!("At most {MAX_NUMBERS} numbers at a time.")));
            }
            let mut draw: Box<dyn FnMut(&mut Rng) -> f64> = match d {
                Distribution::Uniform { from, to } => {
                    let (a, b) = (*from, *to);
                    if !a.is_finite() || !b.is_finite() {
                        return Err(input("Enter the range as two numbers."));
                    }
                    Box::new(move |r| r.uniform_between(a, b))
                }
                Distribution::Normal { mean, sd } => {
                    let (m, s) = (*mean, *sd);
                    if !(s >= 0.0 && s.is_finite()) || !m.is_finite() {
                        return Err(input("The standard deviation must be a non-negative number."));
                    }
                    Box::new(move |r| r.normal(m, s))
                }
                Distribution::Bernoulli { p } => {
                    check_p(*p)?;
                    let p = *p;
                    Box::new(move |r| r.bernoulli(p))
                }
                Distribution::Binomial { p, trials } => {
                    check_p(*p)?;
                    if *trials == 0 {
                        return Err(input("The number of trials must be at least 1."));
                    }
                    let t = Table::binomial(*p, *trials);
                    Box::new(move |r| t.sample(r))
                }
                Distribution::Poisson { lambda } => {
                    if !(*lambda > 0.0 && lambda.is_finite() && *lambda <= 1e6) {
                        return Err(input("Lambda must be a positive number."));
                    }
                    let t = Table::poisson(*lambda);
                    Box::new(move |r| t.sample(r))
                }
                Distribution::Discrete { values, probs } => {
                    if values.is_empty() || values.len() != probs.len() {
                        return Err(input("Give a value and a probability in each row of the input range."));
                    }
                    if probs.iter().any(|p| !(0.0..=1.0).contains(p)) || (probs.iter().sum::<f64>() - 1.0).abs() > 1e-9 {
                        return Err(input("The probabilities must be between 0 and 1 and add up to 1."));
                    }
                    let (v, p) = (values.clone(), probs.clone());
                    Box::new(move |r| r.discrete(&v, &p))
                }
                Distribution::Patterned { .. } => Box::new(|_| 0.0),
            };
            // Row by row, like filling the output range.
            let mut cols = vec![Vec::with_capacity(count); variables];
            for _ in 0..count {
                for c in cols.iter_mut() {
                    c.push(draw(&mut rng));
                }
            }
            cols
        }
    };
    let mut out = Out::default();
    for (j, col) in columns.iter().enumerate() {
        for (i, v) in col.iter().enumerate() {
            out.num(i as u32, j as u32, *v, Look::PLAIN);
        }
    }
    Ok(out)
}

/// How to sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    /// Every `period`-th value.
    Periodic(usize),
    /// `n` values drawn at random, with replacement.
    Random(usize),
}

/// Sampling: a column of values taken from the input.
pub fn sample(values: &[f64], how: Sampling, seed: Option<u64>) -> Result<Out> {
    if values.is_empty() {
        return Err(input("The input range has no numbers."));
    }
    let picked: Vec<f64> = match how {
        Sampling::Periodic(p) => {
            if p == 0 || p > values.len() {
                return Err(input("The period must be between 1 and the number of input values."));
            }
            values.iter().skip(p - 1).step_by(p).copied().collect()
        }
        Sampling::Random(n) => {
            if n == 0 {
                return Err(input("The number of samples must be at least 1."));
            }
            if n > MAX_NUMBERS {
                return Err(Error::TooLarge(format!("At most {MAX_NUMBERS} samples.")));
            }
            let mut rng = match seed {
                Some(s) => Rng::new(s),
                None => Rng::from_time(),
            };
            (0..n).filter_map(|_| values.get(rng.below(values.len() as u64) as usize).copied()).collect()
        }
    };
    let mut out = Out::default();
    for (i, v) in picked.iter().enumerate() {
        out.num(i as u32, 0, *v, Look::PLAIN);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolpak::V;

    fn column(out: &Out, c: u32) -> Vec<f64> {
        let mut v: Vec<(u32, f64)> = out.cells.iter().filter(|x| x.1 == c).map(|x| (x.0, if let V::Num(n) = x.2 { n } else { f64::NAN })).collect();
        v.sort_by_key(|x| x.0);
        v.into_iter().map(|x| x.1).collect()
    }

    #[test]
    fn seeded_and_shaped() {
        let d = Distribution::Uniform { from: 5.0, to: 6.0 };
        let a = random_numbers(2, 50, &d, Some(7)).unwrap();
        assert_eq!(a, random_numbers(2, 50, &d, Some(7)).unwrap(), "same seed, same numbers");
        assert_eq!(a.extent(), (50, 2));
        assert!(column(&a, 1).iter().all(|x| (5.0..=6.0).contains(x)));
        let b = random_numbers(1, 2000, &Distribution::Bernoulli { p: 0.25 }, Some(1)).unwrap();
        let ones = column(&b, 0).iter().filter(|x| **x == 1.0).count();
        assert!((400..600).contains(&ones), "{ones}");
        let n = random_numbers(1, 5000, &Distribution::Normal { mean: 3.0, sd: 0.5 }, Some(2)).unwrap();
        let m = column(&n, 0).iter().sum::<f64>() / 5000.0;
        assert!((m - 3.0).abs() < 0.05);
        let d = Distribution::Discrete { values: vec![10.0, 20.0], probs: vec![0.5, 0.5] };
        assert!(column(&random_numbers(1, 100, &d, Some(3)).unwrap(), 0).iter().all(|x| *x == 10.0 || *x == 20.0));
        let bin = random_numbers(1, 100, &Distribution::Binomial { p: 0.5, trials: 10 }, Some(4)).unwrap();
        assert!(column(&bin, 0).iter().all(|x| (0.0..=10.0).contains(x) && x.fract() == 0.0));
        let poi = random_numbers(1, 100, &Distribution::Poisson { lambda: 2.0 }, Some(5)).unwrap();
        assert!(column(&poi, 0).iter().all(|x| *x >= 0.0 && x.fract() == 0.0));
    }

    #[test]
    fn patterned_and_errors() {
        let d = Distribution::Patterned { from: 1.0, to: 3.0, step: 1.0, repeat_each: 2, repeat_sequence: 2 };
        let out = random_numbers(1, 0, &d, None).unwrap();
        assert_eq!(column(&out, 0), vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
        assert!(random_numbers(1, 10, &Distribution::Bernoulli { p: 2.0 }, None).is_err());
        assert!(random_numbers(1, 10, &Distribution::Discrete { values: vec![1.0], probs: vec![0.5] }, None).is_err());
        assert!(random_numbers(0, 10, &Distribution::Uniform { from: 0.0, to: 1.0 }, None).is_err());
        assert!(random_numbers(2000, 1000, &Distribution::Uniform { from: 0.0, to: 1.0 }, None).is_err());
        let bad = Distribution::Patterned { from: 3.0, to: 1.0, step: 1.0, repeat_each: 1, repeat_sequence: 1 };
        assert!(random_numbers(1, 0, &bad, None).is_err());
    }

    #[test]
    fn sampling() {
        let x: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(column(&sample(&x, Sampling::Periodic(3), None).unwrap(), 0), vec![3.0, 6.0, 9.0]);
        let r = sample(&x, Sampling::Random(20), Some(9)).unwrap();
        assert_eq!(column(&r, 0).len(), 20);
        assert!(column(&r, 0).iter().all(|v| x.contains(v)));
        assert!(sample(&x, Sampling::Periodic(11), None).is_err());
        assert!(sample(&[], Sampling::Random(1), None).is_err());
    }
}
