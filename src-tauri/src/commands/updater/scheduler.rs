//! Auto-check scheduler + `update_relaunch` IPC.
//!
//! The scheduler wakes once every [`AUTO_CHECK_INTERVAL`] (24h by
//! default), re-reads the live settings on every wake (so a user who
//! toggles auto-check off mid-run is honoured on the next wake), and
//! fires `run_check` only when [`should_auto_check`] returns true.
//! Failures trigger the backoff sequence 1h → 6h → 24h.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::error::AppError;
use crate::state::AppState;

#[cfg(not(test))]
use super::backend::PluginBackend;
#[cfg(not(test))]
use super::checker::run_check;
use super::types::UpdaterState;

/// Minimum wall-clock interval between auto-checks, regardless of how
/// often the app is restarted. 24h matches Sparkle's default + macOS
/// App Store cadence (see Phase 15 plan §Resolved Decision #3).
pub const AUTO_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Backoff steps when an auto-check fails. Order is 1h → 6h → 24h;
/// after the third failure the next attempt waits a full 24h window.
///
/// `#[allow(dead_code)]` because the constant is only read inside the
/// `#[cfg(not(test))]` branch of `spawn_auto_check_scheduler` — the
/// test build legitimately doesn't reach it. Pinned by
/// `auto_check_backoff_sequence_matches_plan_spec` so the values are
/// still test-covered.
#[allow(dead_code)]
pub const AUTO_CHECK_BACKOFF: &[Duration] = &[
    Duration::from_secs(60 * 60),
    Duration::from_secs(6 * 60 * 60),
    Duration::from_secs(24 * 60 * 60),
];

/// Decide whether the auto-check scheduler should fire right now.
/// Pure function — extracted so the schedule logic can be unit-tested
/// without an `AppState`, network, or filesystem.
///
/// Returns `true` when **all** of:
/// - `auto_check_enabled` is true (settings opt-in)
/// - `paranoid_mode` is false (kill switch off)
/// - The time since `last_checked_at` is at least `AUTO_CHECK_INTERVAL`,
///   OR `last_checked_at` is `None` (never checked before).
#[allow(dead_code)]
pub fn should_auto_check(
    auto_check_enabled: bool,
    paranoid_mode: bool,
    last_checked_at: Option<i64>,
    now: i64,
) -> bool {
    if !auto_check_enabled || paranoid_mode {
        return false;
    }
    match last_checked_at {
        None => true,
        Some(prev) => {
            let elapsed_secs = now.saturating_sub(prev);
            elapsed_secs >= AUTO_CHECK_INTERVAL.as_secs() as i64
        }
    }
}

/// Relaunch the running app process so a freshly-installed update
/// becomes the active binary. The plugin's macOS install path replaces
/// the .app bundle but does not auto-restart the running process; this
/// command bridges that gap.
///
/// The restart itself is fired on a short delay so the IPC response
/// arrives at the renderer before the process dies. `tauri::AppHandle::
/// restart()` is `-> !` — calling it directly from the command body
/// would tear the IPC socket down mid-response.
///
/// No `require_network` gate: this is a purely local process action.
#[tauri::command]
pub async fn update_relaunch(app: tauri::AppHandle) -> Result<(), AppError> {
    #[cfg(test)]
    {
        let _ = app;
        Err(AppError::Internal {
            message: "update_relaunch is not callable in tests".into(),
        })
    }
    #[cfg(not(test))]
    {
        tauri::async_runtime::spawn(async move {
            // 50ms is enough for the JSON IPC response to make it to
            // the renderer before the process restarts.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            app.restart();
        });
        Ok(())
    }
}

/// Spawn the auto-check scheduler as a tokio background task. Called
/// once at app startup from `state::initialize`. The task:
///
/// 1. Sleeps for [`AUTO_CHECK_INTERVAL`].
/// 2. Reads the live settings (re-reads each cycle so a user who
///    toggles auto-check off mid-run is honoured on the next wake).
/// 3. If `should_auto_check` returns true, runs `run_check`. Failures
///    trigger the backoff sequence; successes reset to the 24h cadence.
/// 4. Loops forever.
pub fn spawn_auto_check_scheduler<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    #[cfg(not(test))]
    {
        use tauri::Manager;
        tauri::async_runtime::spawn(async move {
            // On the first wake we still defer one interval. Rationale:
            // the manual button is one click away in Settings, so the
            // user who wants an immediate check at launch has a path;
            // the auto-check is for *unattended* update discovery, not
            // "ping the endpoint the moment the app opens".
            let mut sleep_for = AUTO_CHECK_INTERVAL;
            let mut backoff_idx = 0usize;
            loop {
                tokio::time::sleep(sleep_for).await;

                let state: tauri::State<AppState> = app.state();
                let (auto_on, paranoid_on, last_checked_at) = read_scheduler_inputs(&state).await;
                let now = chrono::Utc::now().timestamp();

                if !should_auto_check(auto_on, paranoid_on, last_checked_at, now) {
                    // Try again at the canonical cadence — we don't
                    // escalate sleep here because the user could flip
                    // the toggle on at any point.
                    sleep_for = AUTO_CHECK_INTERVAL;
                    backoff_idx = 0;
                    continue;
                }

                let backend = PluginBackend::new(app.clone());
                match run_check(&state, &backend).await {
                    Ok(_) => {
                        sleep_for = AUTO_CHECK_INTERVAL;
                        backoff_idx = 0;
                    }
                    Err(e) => {
                        tracing::warn!("updater: auto-check failed (non-fatal): {e:?}");
                        let next = AUTO_CHECK_BACKOFF
                            .get(backoff_idx)
                            .copied()
                            .unwrap_or(AUTO_CHECK_INTERVAL);
                        sleep_for = next;
                        backoff_idx = (backoff_idx + 1).min(AUTO_CHECK_BACKOFF.len() - 1);
                    }
                }
            }
        });
    }
    #[cfg(test)]
    {
        // Under cfg(test) we don't spawn the real scheduler — its loop
        // body references the PluginBackend which is itself cfg-gated
        // out so it can't be instantiated. Tests exercise the pure
        // `should_auto_check` gate function directly and the IPC
        // commands through the trait-injected mock backend.
        let _ = app;
    }
}

/// Read the three settings the scheduler needs in a single guard
/// acquisition. Returns `(auto_check_enabled, paranoid_mode, last_checked_at)`.
#[cfg(not(test))]
async fn read_scheduler_inputs(state: &AppState) -> (bool, bool, Option<i64>) {
    use crate::commands::settings::SettingsLoadState;
    let (auto_on, paranoid_on) = {
        let guard = state.settings.read().await;
        match &*guard {
            SettingsLoadState::Loaded(s) => (s.update_auto_check, s.paranoid_mode),
            SettingsLoadState::FirstLaunch => (false, false),
            // Corrupt: deny outbound. The require_network gate inside
            // run_check would also catch this, but short-circuiting here
            // saves us a wakeup + a network attempt.
            SettingsLoadState::Corrupt { .. } => (false, true),
        }
    };
    let last_checked_at = {
        let guard = state.updater_state.read().await;
        guard.last_checked_at
    };
    (auto_on, paranoid_on, last_checked_at)
}

/// Re-export so `AppState::build()` can construct the wrapped state slot.
pub fn empty_state() -> Arc<RwLock<UpdaterState>> {
    Arc::new(RwLock::new(UpdaterState::default()))
}
