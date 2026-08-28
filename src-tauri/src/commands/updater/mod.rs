//! Phase 15 — in-app updater commands (refactored into submodules).
//!
//! Module layout (each file is small, single-purpose, and unit-testable):
//!
//! - `types` — wire types (UpdateCheckOutcome, CachedUpdate, UpdaterState),
//!   plus `parse_semver` / `is_strict_upgrade` / `current_app_version`.
//! - `backend` — `UpdaterBackend` trait + `PluginBackend` production impl.
//! - `checker` — `run_check` + `update_check_now` IPC + `is_version_skipped`.
//! - `installer` — `run_install`, `run_skip` + `update_install` / `update_skip` IPC.
//! - `scheduler` — `should_auto_check`, `spawn_auto_check_scheduler`,
//!   `update_relaunch` IPC, `empty_state` helper.
//!
//! ## Why this refactor
//!
//! The original `commands/updater.rs` was 51 KB and mixed:
//!   - IPC command bodies (4 #[tauri::command] fns)
//!   - plugin-frontend (PluginBackend)
//!   - plugin-error translation
//!   - semver parsing
//!   - scheduler state machine
//!   - per-launch defaults
//!
//! Splitting lets reviewers find any one concern in one short file, and
//! keeps each test module focused on a single behaviour. Public API
//! (the IPC names) is unchanged — `lib.rs`'s `tauri::generate_handler!`
//! list still references `commands::updater::update_check_now` etc.
//! exactly as before.

pub mod backend;
pub mod checker;
pub mod installer;
pub mod scheduler;
pub mod types;

#[cfg(test)]
mod tests;

// Public re-exports — the lib.rs `generate_handler!` list and the rest
// of the app import these by their previous, flat paths.
pub use backend::UpdaterBackend;
pub use checker::{run_check, update_check_now};
pub use installer::{run_install, run_skip, update_install, update_skip};
pub use scheduler::{empty_state, spawn_auto_check_scheduler, update_relaunch};
pub use types::{
    current_app_version, is_strict_upgrade, CachedUpdate, UpdateCheckOutcome, UpdaterState,
};
