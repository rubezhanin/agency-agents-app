//! Catalog source persistence and resolution.
//!
//! A `CatalogSource` records WHERE the catalog is read from at
//! runtime: the bundled baseline (always-works default), a managed
//! local clone under `~/.agency-agents` that the app keeps in sync
//! via `git pull`, or a user-selected clone (which the user is
//! responsible for keeping up to date). The choice is persisted to
//! `<app_data>/catalog.json` and read on every cold start.
//!
//! Extracted from `corpus/mod.rs` so the catalog state machine is
//! reviewable on its own — the `Corpus` struct + `resolve_active`
//! live in `corpus/mod.rs`, but the source-selection plumbing is
//! independent and tests well in isolation.

use std::path::{Path, PathBuf};

use super::paths::{corpus_dir, state_dir};
use crate::error::AppError;
use crate::types::CatalogSource;
use crate::util::fs::atomic_write;

/// Path to the persisted `CatalogSource` JSON (`<app_data>/catalog.json`).
fn catalog_source_path(app_data_dir: &Path) -> PathBuf {
    state_dir(app_data_dir).join("catalog.json")
}

/// Load the persisted [`CatalogSource`], or [`CatalogSource::Bundled`] when no
/// file exists yet. Never panics on a corrupt file — the worst case is the
/// default, which is also what the user gets on first launch.
pub(crate) async fn load_catalog_source(app_data_dir: &Path) -> CatalogSource {
    let path = catalog_source_path(app_data_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => CatalogSource::default(),
    }
}

/// Persist the chosen [`CatalogSource`] to `state/catalog.json`. Atomic
/// write — temp + fsync + rename — so a crash mid-write leaves the
/// previous choice intact.
pub(crate) async fn save_catalog_source(
    app_data_dir: &Path,
    source: &CatalogSource,
) -> Result<(), AppError> {
    if let Some(parent) = catalog_source_path(app_data_dir).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create state dir {}: {e}", parent.display()),
            })?;
    }
    let bytes = serde_json::to_vec_pretty(source).map_err(|e| AppError::Io {
        message: format!("serialize catalog source: {e}"),
    })?;
    atomic_write(&catalog_source_path(app_data_dir), &bytes)
        .await
        .map_err(|e| AppError::Io {
            message: format!("write catalog source: {e}"),
        })?;
    // Best-effort fsync of the parent dir so the write is durable
    // across a hard power-cycle (POSIX behaviour; on Windows
    // opening the dir as a file is harmless even if it doesn't
    // round-trip through `OpenOptions`).
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(app_data_dir) {
        let _ = dir.sync_all();
    }
    // Touch the app_data_dir side to keep Clippy happy about the
    // `&[u8]` import path being otherwise unused in the test build.
    let _ = bytes.first();
    Ok(())
}

/// Resolve the on-disk path where catalog content lives for `source`.
///
/// - `Bundled` → the app-managed baseline (always works).
/// - `Managed { path }` → that exact path (the user's
///   `~/.agency-agents`).
/// - `UserClone { path, .. }` → that exact path.
pub(crate) fn catalog_root(app_data_dir: &Path, source: &CatalogSource) -> PathBuf {
    match source {
        CatalogSource::Bundled => corpus_dir(app_data_dir),
        CatalogSource::Managed { path } => PathBuf::from(path),
        CatalogSource::UserClone { path, .. } => PathBuf::from(path),
    }
}
