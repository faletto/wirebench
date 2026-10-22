//! Startup timing, so "how long until it is usable" is a number on screen, not a guess.
//!
//! Web: `performance.now()` is milliseconds since navigation began, so the figure includes
//! download, wasm compile and app construction. Native: milliseconds since [`mark_start`].

/// Native only: call first thing in `main` so the figure covers app construction.
#[cfg(not(target_arch = "wasm32"))]
pub fn mark_start() {
    let _ = START.set(std::time::Instant::now());
}

#[cfg(not(target_arch = "wasm32"))]
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
pub fn since_start_ms() -> Option<f64> {
    // If `mark_start` was never called (tests), the clock starts at first use.
    Some(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_secs_f64()
            * 1000.0,
    )
}

#[cfg(target_arch = "wasm32")]
pub fn since_start_ms() -> Option<f64> {
    web_sys::window()?.performance().map(|p| p.now())
}

#[cfg(target_arch = "wasm32")]
pub fn log(msg: &str) {
    web_sys::console::log_1(&msg.into());
}

#[cfg(not(target_arch = "wasm32"))]
pub fn log(msg: &str) {
    eprintln!("{msg}");
}
