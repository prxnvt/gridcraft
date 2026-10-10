//! GridCraft analysis maths: the numeric side of the Analysis ToolPak and Solver, kept free of
//! the workbook model so it is easy to test and runs anywhere (including wasm).
//!
//! - [`linalg`]: dense LU factorisation (solve, inverse, determinant).
//! - [`fft`]: radix-2 fast Fourier transform with the ToolPak's scaling.
//! - [`rng`]: a seedable xoshiro256** generator and the ToolPak's distributions.
//!
//! Distributions and descriptive statistics come from `gridcraft_functions::dist`, so results
//! match the worksheet functions exactly.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod fft;
pub mod linalg;
pub mod rng;
pub mod toolpak;

/// What can go wrong in an analysis.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Error {
    /// The input doesn't suit the analysis (message for the user).
    #[error("{0}")]
    Input(String),
    /// The input exceeds a size limit.
    #[error("{0}")]
    TooLarge(String),
    /// The computation has no finite answer (singular matrix, zero variance…).
    #[error("{0}")]
    Numeric(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Helper: an input error.
pub fn input(msg: impl Into<String>) -> Error {
    Error::Input(msg.into())
}
