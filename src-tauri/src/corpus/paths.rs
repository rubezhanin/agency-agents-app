//! Path helpers for the corpus subsystem.
//!
//! All derived from `app_data_dir` — never composed from IPC input. Centralising
//! them here means there is exactly one place to look when a layout change
//! touches `<app_data>/corpus` or `<app_data>/state`.

use std::path::{Path, PathBuf};

/// The working corpus directory: `<app_data_dir>/corpus`. ALWAYS derived
/// from `app_data_dir` — never composed from IPC input.
pub(crate) fn corpus_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("corpus")
}

/// The state directory holding `corpus-index.json` + `corpus-meta.json` and
/// (Phase 2) the install ledger `installs.json`.
pub(crate) fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("state")
}

pub(crate) fn index_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("corpus-index.json")
}

pub(crate) fn meta_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("corpus-meta.json")
}

pub(crate) fn catalog_source_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("catalog.json")
}
