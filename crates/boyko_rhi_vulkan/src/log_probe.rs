//! This crate's binding of [`boyko_log::probe`] to its own [`boyko_log::RhiVulkan`] target.
//!
//! **The implementation moved out at rung L8a.** L7b wrote the four helpers here, privately,
//! because a `#[cfg(test)]` item in `boyko_log` is invisible to a crate that links the library
//! compiled without `cfg(test)`. L8a needed the same four in four more crates, so the helpers
//! became `boyko_log::probe` behind the `test-probe` feature (enabled here in
//! `[dev-dependencies]` only) and this file became the one line that binds them to a target.
//! Everything L7b learned — why the observers serialize on a lock, why `observed` must count both
//! delivery routes, and why the reporters stay private so the COMPILER gates their wiring — is in
//! that module's header now, in one copy.

pub(crate) use boyko_log::probe::{drain, observe_lock};

/// Records this process has produced on the `RhiVulkan` target, both routes.
pub(crate) fn observed() -> u64 {
    boyko_log::probe::observed::<boyko_log::RhiVulkan>()
}

/// Raise the `RhiVulkan` ceiling so a `Warn` is admitted. Called **before** the emission.
pub(crate) fn arm() {
    boyko_log::probe::arm::<boyko_log::RhiVulkan>();
}
