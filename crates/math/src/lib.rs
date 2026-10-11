//! GridCraft equations. LaTeX source is laid out by RaTeX (`ratex-parser`, `ratex-layout`; MIT)
//! into plain data that the UI and the PDF writer paint without calling RaTeX again.
//!
//! RaTeX is young, so it only ever runs inside this crate's guarded entry points; see the M14.28
//! gate in `docs/decisions.md` and `tests/gate.rs`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
