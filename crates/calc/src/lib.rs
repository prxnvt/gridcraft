//! GridCraft's calculation engine: the evaluator (references, names, tables, LET/LAMBDA,
//! dynamic arrays) and dependency-driven recalculation with spills and cycle detection.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod eval;
pub mod recalc;

pub use eval::{Area, SPECIAL_FUNCTIONS, is_known_function, precedents};
pub use recalc::{Calc, Key, MAX_TABLE_CELLS, Trial, evaluate, evaluate_expr, lay_python_spill, now_serial};

#[cfg(test)]
mod tests;
