//! Tauri command surface for settings — read / set / reset / app version.

use std::path::Path;

use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

use super::load::{persist, settings_path, update};
use super::types::{Settings, SettingsLoadState};

/// Read the current settings. Returns the in-memory cached struct
/// (post-clamp). `FirstLaunch` is treated as "return defaults" so the
/// UI never has to special-case it. `Corrupt` surfaces as an
/// `AppError::Internal` with the original corruption message so the
/// UI can route the user to `settings_reset`.
#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> Result<Settings, AppError> {
    let guard = state.settings.read().await;
    match &*guard {
        SettingsLoadState::Loaded(s) => Ok(s.clone()),
        SettingsLoadState::FirstLaunch => Ok(Settings::default()),
        SettingsLoadState::Corrupt { message } => Err(AppError::Internal {
            message: format!("settings file is unreadable: {message}"),
        }),
    }
}

/// Write a complete settings struct to disk and update the in-memory
/// cache. The frontend always sends a complete object (merging with
/// existing values is the caller's responsibility, not ours).
#[tauri::command]
pub async fn settings_set(
    settings: Settings,
    state: State<'_, AppState>,
) -> Result<Settings, AppError> {
    let clamped = persist(&state.app_data_dir, settings).await?;
    {
        let mut guard = state.settings.write().await;
        *guard = SettingsLoadState::Loaded(clamped.clone());
    }
    Ok(clamped)
}

/// Overwrite `settings.json` with the defaults and update the
/// in-memory cache. Used by the UI's "Reset to defaults" button when
/// the file is corrupt or the user just wants to start fresh.
#[tauri::command]
pub async fn settings_reset(state: State<'_, AppState>) -> Result<Settings, AppError> {
    let defaults = Settings::default();
    let clamped = persist(&state.app_data_dir, defaults).await?;
    {
        let mut guard = state.settings.write().await;
        *guard = SettingsLoadState::Loaded(clamped.clone());
    }
    Ok(clamped)
}

/// Return the app's version string from the Tauri package info. Source of
/// truth is `Cargo.toml` (`tauri.conf.json` mirrors it). Avoids reading
/// `package.json` from the renderer.
#[tauri::command]
pub fn app_version<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> String {
    app.package_info().version.to_string()
}

/// Apply a mutator to the loaded settings and persist. Used by the
/// auto-check scheduler / the run_skip helper in
/// `commands::updater` for incremental updates that don't need a
/// full `Settings` from the renderer.
#[allow(dead_code)]
pub async fn settings_update<F>(app_data_dir: &Path, mutate: F) -> Result<Settings, AppError>
where
    F: FnOnce(&mut Settings) + Send,
{
    update(app_data_dir, mutate).await
}
