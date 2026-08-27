//! Stats + status IPC: `github_repo_stats` + the two paths that don't
//! require a signed-in token.
//!
//! `github_repo_stats` is the highest-traffic GitHub call — every
//! PackageDetail row hits it on scroll. Three gate conditions collapse
//! the answer to `Ok(None)` without any outbound call:
//!
//! 1. Settings opt-in gate. `github_enabled` defaults OFF. The user
//!    has to explicitly opt in before we touch the network.
//! 2. Paranoid-mode gate. Master switch wins even with the opt-in ON.
//! 3. URL allowlist. Non-GitHub homepages are treated as "no stats"
//!    (the same shape the frontend expects when stats are simply
//!    unavailable).

use tauri::State;

use crate::commands::settings::SettingsLoadState;
use crate::error::AppError;
use crate::github::{self, auth, fetch_repo_stats, parse_github_url, GithubStatusDto, RepoStats};
use crate::state::AppState;

/// Read the GitHub auth status from the Keychain. No network call.
#[tauri::command]
pub async fn github_status(_state: State<'_, AppState>) -> Result<GithubStatusDto, AppError> {
    auth::status()
}

/// Fetch live stats for a GitHub repo URL. The three gates are
/// short-circuit `Ok(None)` to keep the PackageDetail's hot path
/// network-free when the user hasn't opted in.
#[tauri::command]
pub async fn github_repo_stats(
    homepage: String,
    state: State<'_, AppState>,
) -> Result<Option<RepoStats>, AppError> {
    // 1. Settings opt-in gate.
    {
        let guard = state.settings.read().await;
        let enabled = match &*guard {
            SettingsLoadState::Loaded(s) => s.github_enabled,
            // First launch defaults: github_enabled = false.
            SettingsLoadState::FirstLaunch => false,
            // Corrupt → fail closed.
            SettingsLoadState::Corrupt { .. } => false,
        };
        if !enabled {
            return Ok(None);
        }
    }

    // 2. Paranoid-mode gate.
    state.require_network("github_repo_stats").await?;

    // 3. URL allowlist. Non-github URLs collapse to None.
    let repo = match parse_github_url(&homepage) {
        Some(r) => r,
        None => return Ok(None),
    };

    // 4. Issue the fetch.
    let client = github::stats::build_client()?;
    let auth_token = auth::read_token()?;
    let cache_dir = state.app_data_dir.join("github-cache");
    fetch_repo_stats(&client, &repo, auth_token.as_ref(), &cache_dir).await
}
