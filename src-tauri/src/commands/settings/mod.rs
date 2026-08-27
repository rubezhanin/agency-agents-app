//! Settings persistence (Phase 12d), split into submodules.
//!
//! Module layout (each file is small, single-purpose, and unit-testable):
//!
//! - `types` — `Settings` + `CaskIconMode` + `SettingsLoadState` + the
//!   `clamp` / `push_skipped_version` helpers. Pure data.
//! - `load` — `settings_path` / `load_at_startup` / `load_async` /
//!   `persist` / `update` / `read_raw` / `write_raw`. All file IO
//!   lives here.
//! - `commands` — IPC: `settings_get` / `settings_set` /
//!   `settings_reset` / `app_version` / `settings_update`.
//! - `tests` — integration tests for the load + persist + clamp flow.
//!
//! ## Why this refactor
//!
//! The original `commands/settings.rs` was 47 KB and mixed:
//!   - the `Settings` schema (data)
//!   - the `SettingsLoadState` enum (also data, but tied to the
//!     `require_network` security gate that lives elsewhere)
//!   - every persistence routine (file IO, clamp on read/write)
//!   - the four `#[tauri::command]` IPC bodies
//!   - the full test submodule (~30 tests)
//!
//! Splitting lets reviewers find any one concern in one short file
//! and keeps each test focused on a single behaviour. The IPC
//! contract is unchanged — `lib.rs`'s `generate_handler!` list still
//! references `commands::settings::settings_get` etc.

pub mod commands;
pub mod load;
pub mod types;

#[cfg(test)]
mod tests;

// Public re-exports — the lib.rs `generate_handler!` list and the rest
// of the app import these by their previous, flat paths.
pub use commands::{app_version, settings_get, settings_reset, settings_set, settings_update};
pub use load::{load_async, load_at_startup, persist, settings_path, update};
pub use types::{CaskIconMode, Settings, SettingsLoadState, MAX_SETTINGS_BYTES};
