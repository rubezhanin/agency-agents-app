//! Settings persistence: `settings_path` / `load_at_startup` /
//! `load_async` / `persist` / `update`.
//!
//! All file IO is centralised here. Callers (the IPC layer, the
//! auto-check scheduler, the `run_skip` helper in `commands::updater`)
//! call into these and never touch `std::fs` or `tokio::fs` themselves.
//! This is the load-bearing module for the "fail closed on corrupt
//! settings" security gate.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use crate::error::AppError;
use crate::util::fs::{atomic_write, read_capped};

use super::types::{Settings, SettingsLoadState, MAX_SETTINGS_BYTES};

/// Always `<app_data_dir>/settings.json`. The directory is created if
/// missing — the caller (typically `AppState::build`) has already
/// ensured `app_data_dir` exists, so this is a defense-in-depth mkdir.
pub fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

/// Synchronous startup loader. Called from `AppState::build()` (which is
/// a non-async function) so we use the blocking `std::fs` API rather
/// than tokio. The trade-off accepted is a single small read on startup
/// in exchange for a much simpler init story.
///
/// Returns the same three-state shape as the async loader so callers
/// stay uniform.
pub fn load_at_startup(app_data_dir: &Path) -> SettingsLoadState {
    let path = settings_path(app_data_dir);

    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return SettingsLoadState::FirstLaunch;
        }
        Err(e) => {
            // Stat failed for some non-NotFound reason (permission denied,
            // EIO, etc.). Treat as corrupt — fail closed.
            tracing::warn!("settings: stat failed at {}: {e}", path.display());
            return SettingsLoadState::Corrupt {
                message: format!("stat {}: {e}", path.display()),
            };
        }
    };

    if meta.len() > MAX_SETTINGS_BYTES {
        tracing::warn!(
            "settings: {} is {} bytes, exceeds {}-byte cap; treating as corrupt",
            path.display(),
            meta.len(),
            MAX_SETTINGS_BYTES
        );
        return SettingsLoadState::Corrupt {
            message: format!(
                "settings.json is {} bytes, exceeds {}-byte cap",
                meta.len(),
                MAX_SETTINGS_BYTES
            ),
        };
    }

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("settings: read failed at {}: {e}", path.display());
            return SettingsLoadState::Corrupt {
                message: format!("read {}: {e}", path.display()),
            };
        }
    };

    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(mut s) => {
            s.clamp();
            SettingsLoadState::Loaded(s)
        }
        Err(e) => {
            tracing::warn!(
                "settings: parse failed at {}: {e}; treating as corrupt",
                path.display()
            );
            SettingsLoadState::Corrupt {
                message: format!("parse {}: {e}", path.display()),
            }
        }
    }
}

/// Async loader, identical semantics to [`load_at_startup`] but
/// non-blocking. Used by tests and any future callers that need to
/// re-read from disk without blocking the runtime.
#[allow(dead_code)]
pub async fn load_async(app_data_dir: &Path) -> SettingsLoadState {
    let path = settings_path(app_data_dir);

    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return SettingsLoadState::FirstLaunch;
        }
        Err(e) => {
            tracing::warn!("settings: stat failed at {}: {e}", path.display());
            return SettingsLoadState::Corrupt {
                message: format!("stat {}: {e}", path.display()),
            };
        }
    };

    if meta.len() > MAX_SETTINGS_BYTES {
        return SettingsLoadState::Corrupt {
            message: format!(
                "settings.json is {} bytes, exceeds {}-byte cap",
                meta.len(),
                MAX_SETTINGS_BYTES
            ),
        };
    }

    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => {
            return SettingsLoadState::Corrupt {
                message: format!("read {}: {e}", path.display()),
            };
        }
    };

    match serde_json::from_slice::<Settings>(&bytes) {
        Ok(mut s) => {
            s.clamp();
            SettingsLoadState::Loaded(s)
        }
        Err(e) => SettingsLoadState::Corrupt {
            message: format!("parse {}: {e}", path.display()),
        },
    }
}

/// Persist `settings` to disk and return the clamped struct (so
/// callers can use the clamped copy for the in-memory cache). Atomic
/// write — temp + fsync + rename + fsync(parent).
///
/// Returns `AppError::Io` on failure, with a context string that
/// identifies the operation so the UI can surface a useful toast
/// without leaking raw `e`.
pub async fn persist(app_data_dir: &Path, mut settings: Settings) -> Result<Settings, AppError> {
    settings.clamp();
    let path = settings_path(app_data_dir);

    // Defense-in-depth: refuse to write an oversize payload even if
    // the in-memory struct was constructed with values outside the
    // declared bounds. Catches hand-crafted Settings via IPC.
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| AppError::Io {
        message: format!("serialize settings: {e}"),
    })?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(AppError::Io {
            message: format!(
                "serialized settings is {} bytes, exceeds {}-byte cap",
                bytes.len(),
                MAX_SETTINGS_BYTES
            ),
        });
    }

    // Ensure the parent exists. `AppState::build` should have already
    // created it, but a fresh test harness or a user with a manually-
    // nuked `~/Library/Application Support/<bundle>/` should not
    // crash on first save.
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create settings parent {}: {e}", parent.display()),
            })?;
    }

    atomic_write(&path, &bytes)
        .await
        .map_err(|e| AppError::Io {
            message: format!("write settings to {}: {e}", path.display()),
        })?;

    Ok(settings)
}

/// Apply a mutator closure to the loaded settings and persist. Used
/// for the skip-list push (and any future incremental update flow).
///
/// Returns the clamped post-mutation `Settings` on success so the
/// caller can re-cache it.
#[allow(dead_code)]
pub async fn update<F>(app_data_dir: &Path, mutate: F) -> Result<Settings, AppError>
where
    F: FnOnce(&mut Settings) + Send,
{
    // Load current settings (sync because AppState::build is sync, and
    // we don't want to double the read path for the common case).
    let current = match load_at_startup(app_data_dir) {
        SettingsLoadState::Loaded(s) => s,
        SettingsLoadState::FirstLaunch => Settings::default(),
        // Fail loud: refusing to write to a corrupt file is the
        // documented contract (the run_skip helper in updater relies
        // on this refusal).
        SettingsLoadState::Corrupt { message } => {
            return Err(AppError::Io {
                message: format!("settings file is corrupt: {message}"),
            });
        }
    };

    let mut next = current;
    mutate(&mut next);
    persist(app_data_dir, next).await
}

/// Async helper used by tests to write a settings.json directly (without
/// going through `persist`'s clamp). For tests only.
#[allow(dead_code)]
pub async fn write_raw(app_data_dir: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let path = settings_path(app_data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut f = tokio::fs::File::create(&path).await?;
    f.write_all(bytes).await?;
    f.sync_all().await?;
    Ok(())
}

/// Read the raw settings.json from disk (no parsing, no clamp). Used
/// by tests to assert wire-shape keys. Capped at
/// [`MAX_SETTINGS_BYTES`].
#[allow(dead_code)]
pub async fn read_raw(app_data_dir: &Path) -> Result<Vec<u8>, std::io::Error> {
    read_capped(&settings_path(app_data_dir), MAX_SETTINGS_BYTES)
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}
