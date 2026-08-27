//! `update_install` + `update_skip` IPC, plus the shared
//! `run_install` / `run_skip` cores.
//!
//! The install path applies three defense-in-depth gates on top of
//! `tauri-plugin-updater`'s own signature verification:
//!
//! 1. **Paranoid mode** — every IPC entry consults `require_network`.
//! 2. **Stale-version sanity check** — `version` must match the
//!    in-memory cached `Available` payload.
//! 3. **Explicit downgrade rejection** — refuses semver-older or
//!    semver-equal targets.
//!
//! See `commands::updater::mod` for the architectural rationale.

use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

use super::backend::UpdaterBackend;
use super::types::{current_app_version, is_strict_upgrade};

/// Inner: run an install via the supplied backend, after validating the
/// caller's `version` arg against the cached `Available` payload and
/// rejecting downgrades.
///
/// `#[allow(dead_code)]` because in `cfg(test)` the IPC `update_install`
/// body is a stub that does not call into this function — the real
/// test surface is via the `MockBackend` trait impl.
#[allow(dead_code)]
pub async fn run_install(
    state: &AppState,
    backend: &dyn UpdaterBackend,
    version: &str,
) -> Result<(), AppError> {
    state.require_network("update_check").await?;

    // 1. Sanity check the caller-supplied version against the in-memory
    // cached `Available` payload. Defends against UI staleness: if the
    // user kept the Settings panel open through an auto-check cycle and
    // the available version changed, the install button fires with the
    // *old* version arg.
    let cached = {
        let guard = state.updater_state.read().await;
        guard.cached_available.clone()
    };
    let cached = cached.ok_or_else(|| AppError::InvalidArgument {
        message: format!(
            "no update available to install; run update_check_now first (requested {version})"
        ),
    })?;
    if cached.version != version {
        return Err(AppError::InvalidArgument {
            message: format!(
                "install version mismatch: requested {version}, cached available is {}",
                cached.version
            ),
        });
    }

    // 2. Explicit downgrade rejection (defense in depth; the plugin's
    // own version comparator already does this, but a future plugin
    // behaviour change cannot reopen the hole if we re-check here).
    let current = current_app_version();
    if !is_strict_upgrade(current, version) {
        return Err(AppError::DowngradeRejected {
            current: current.to_string(),
            target: version.to_string(),
        });
    }

    // 3. Delegate to the plugin via the backend trait. The plugin
    // performs the download + sha256 verification (when the manifest
    // carries a hash) + minisign verification + atomic .app bundle
    // replacement in a single call.
    backend.download_and_install(version).await?;

    // 4. Clear the cached available payload so a re-render of the
    // indicator + the Settings card doesn't re-offer the same
    // install. The new binary is on disk; the only remaining action
    // is the relaunch, which `update_relaunch` handles.
    {
        let mut guard = state.updater_state.write().await;
        guard.cached_available = None;
        guard.last_outcome = Some(super::types::UpdateCheckOutcome::UpToDate);
    }

    Ok(())
}

/// Inner: append `version` to the user's skip-list. Persists to
/// settings.json; the auto-check scheduler consults the list at every
/// run.
pub async fn run_skip(state: &AppState, version: &str) -> Result<(), AppError> {
    use crate::commands::settings::{persist, SettingsLoadState};

    // Validate the version arg — defensive against UI passing garbage.
    if version.trim().is_empty() || version.len() > 64 {
        return Err(AppError::InvalidArgument {
            message: format!("invalid version for skip: {version:?}"),
        });
    }

    // 1) Mutate the settings struct (push_skipped_version handles cap +
    //    dedupe internally). Take a snapshot to hand to persist().
    let updated_settings = {
        let guard = state.settings.read().await;
        match &*guard {
            SettingsLoadState::Loaded(s) => {
                let mut s = s.clone();
                s.push_skipped_version(version.to_string());
                s
            }
            SettingsLoadState::FirstLaunch => {
                // No settings file yet — defaults are correct (paranoid
                // OFF matches "user has never configured anything"),
                // and materializing the file with the skip recorded is
                // the right next step.
                let mut s = crate::commands::settings::Settings::default();
                s.push_skipped_version(version.to_string());
                s
            }
            SettingsLoadState::Corrupt { message } => {
                return Err(AppError::Internal {
                    message: format!(
                        "cannot record update skip while settings file is unreadable \
                         ({message}); reset settings from Settings → Network first"
                    ),
                });
            }
        }
    };
    let clamped = persist(&state.app_data_dir, updated_settings).await?;
    {
        let mut guard = state.settings.write().await;
        *guard = SettingsLoadState::Loaded(clamped);
    }

    // 2) Clear the cached "available" entry when it matches the skipped
    //    version, so subsequent update_check_now() responses + the
    //    title-bar indicator state are coherent. The frontend already
    //    flips its own `available = null` optimistically; this keeps
    //    the backend's view in sync.
    {
        let mut guard = state.updater_state.write().await;
        let should_clear = guard
            .cached_available
            .as_ref()
            .is_some_and(|cached| cached.version == version);
        if should_clear {
            guard.cached_available = None;
        }
    }

    Ok(())
}

/// IPC: install the update. The `version` arg is the version string
/// the UI saw on the `Available` card; the backend re-validates it
/// against the in-memory cache before delegating to the plugin.
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    version: String,
) -> Result<(), AppError> {
    #[cfg(test)]
    {
        let _ = (app, state, version);
        Err(AppError::Internal {
            message: "update_install is not callable in tests; use run_install + MockBackend"
                .into(),
        })
    }
    #[cfg(not(test))]
    {
        let backend = super::backend::PluginBackend::new(app);
        super::installer::run_install(&state, &backend, &version).await
    }
}

/// IPC: add `version` to the skip-list. Used by the title-bar
/// indicator's `×` button.
#[tauri::command]
pub async fn update_skip(version: String, state: State<'_, AppState>) -> Result<(), AppError> {
    super::installer::run_skip(&state, &version).await
}
