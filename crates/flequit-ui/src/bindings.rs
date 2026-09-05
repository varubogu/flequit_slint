//! Re-exports of the types Slint generates from `ui/main.slint`.
//!
//! Keeping the `include_modules!` call in one place means the rest of the crate
//! imports Slint types from `crate::bindings` rather than from an opaque macro
//! expansion, which keeps `cargo doc` and IDE navigation usable.

slint::include_modules!();
