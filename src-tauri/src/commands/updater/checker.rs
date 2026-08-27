//! `update_check_now` IPC + the shared `run_check` core.
//!
//! Pure orchestration: consult the gated network, call the backend,
//! translate to `UpdateCheckOutcome`, persist into `AppState.updater_state`
//! for the install-side sanity check.

use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

use super::backend::UpdaterBackend;
use super::types::{CachedUpdate, UpdateCheckOutcome};

/// Run a single check via the supplied backend, updating
/// `state.updater_state` with the cached result + timestamp. Extracted
/// so the auto-check scheduler can call it without going through the
/// IPC layer.
///
/// `#[allow(dead_code)]` because in `cfg(test)` the IPC `update_check_now`
/// body is a stub that does not call into this function — the real
/// test surface is via the `MockBackend` trait impl. Once a sibling
/// `tests` submodule is split out this allow can be removed.
#[allow(dead_code)]
pub async fn run_check(
    state: &AppState,
    backend: &dyn UpdaterBackend,
) -> Result<UpdateCheckOutcome, AppError> {
    state.require_network("update_check").await?;
    let raw = backend.check().await?;
    let now = chrono::Utc::now().timestamp();

    let outcome = match raw {
        None => UpdateCheckOutcome::UpToDate,
        Some(update) => {
            let skipped = is_version_skipped(state, &update.version).await;
            UpdateCheckOutcome::Available {
                version: update.version.clone(),
                current_version: update.current_version.clone(),
                notes: update.notes.clone(),
                pub_date: update.pub_date.clone(),
                skipped,
            }
        }
    };

    // Persist into AppState so subsequent install requests can validate.
    {
        let mut guard = state.updater_state.write().await;
        guard.last_outcome = Some(outcome.clone());
        guard.last_checked_at = Some(now);
        guard.cached_available = match &outcome {
            UpdateCheckOutcome::UpToDate => None,
            UpdateCheckOutcome::Available {
                version,
                current_version,
                notes,
                pub_date,
                ..
            } => Some(CachedUpdate {
                version: version.clone(),
                current_version: current_version.clone(),
                notes: notes.clone(),
                pub_date: pub_date.clone(),
            }),
        };
    }

    Ok(outcome)
}

/// True iff `version` is in the user's skip-list.
#[allow(dead_code)]
async fn is_version_skipped(state: &AppState, version: &str) -> bool {
    use crate::commands::settings::SettingsLoadState;
    let guard = state.settings.read().await;
    match &*guard {
        SettingsLoadState::Loaded(s) => s.skipped_update_versions.iter().any(|v| v == version),
        _ => false,
    }
}

/// Run a manual update check. Frontend-callable via the "Check for
/// updates now" button in Settings → Network → Updates.
///
/// Returns `UpdateCheckOutcome` on success or `AppError` on failure
/// (including `ParanoidModeBlocked` when Offline Mode is on, surfaced
/// as the typed error rather than a fourth enum variant so the toast
/// channel stays uniform with every other gated call).
#[tauri::command]
pub async fn update_check_now(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateCheckOutcome, AppError> {
    #[cfg(test)]
    {
        let _ = (app, state);
        Err(AppError::Internal {
            message: "update_check_now is not callable in tests; use run_check + MockBackend"
                .into(),
        })
    }
    #[cfg(not(test))]
    {
        let backend = super::backend::PluginBackend::new(app);
        super::checker::run_check(&state, &backend).await
    }
}
