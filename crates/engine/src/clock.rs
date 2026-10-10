//! Wall-clock milliseconds that work everywhere. `std::time::Instant` panics on
//! `wasm32-unknown-unknown`, so the web host installs its own clock with [`set_clock`]; without
//! one, the browser build reports 0 (time limits then fall back to iteration limits).

use std::sync::OnceLock;

static CLOCK: OnceLock<fn() -> f64> = OnceLock::new();

/// Installs the host's millisecond clock (the web app passes `Date.now`). First call wins.
pub fn set_clock(f: fn() -> f64) {
    let _ = CLOCK.set(f);
}

/// Milliseconds since an arbitrary epoch.
pub fn now_ms() -> f64 {
    if let Some(f) = CLOCK.get() {
        return f();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::disallowed_methods)] // never on wasm (see above)
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

/// Whether [`now_ms`] measures real time (false in a browser without a host clock).
pub fn has_clock() -> bool {
    CLOCK.get().is_some() || cfg!(not(target_arch = "wasm32"))
}
