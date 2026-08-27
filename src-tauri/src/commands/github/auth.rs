//! Auth IPC: sign-in start / sign-in poll / sign-out.
//!
//! `github_status` lives in `stats.rs` alongside `github_repo_stats` —
//! they're both zero-network reads from the Keychain. Every command
//! here is gated by `state.require_network(...)` BEFORE the GitHub
//! call lands — paranoid mode must block even the OAuth handshake
//! (per §12d). Sign-out is the only exception: it's a Keychain
//! delete, never outbound.

use tauri::State;

use crate::error::AppError;
use crate::github::{auth, DeviceFlowStart, PollResult, PollResultDto};
use crate::state::AppState;

/// Start the OAuth Device Flow. Returns the user-code + verification
/// URL the user has to enter at github.com/login/device.
#[tauri::command]
pub async fn github_signin_start(state: State<'_, AppState>) -> Result<DeviceFlowStart, AppError> {
    // Sign-in itself is outbound — paranoid mode blocks even the OAuth
    // handshake. Per §12d this is by design: the user can't sign in if
    // they've told us not to make outbound calls.
    state.require_network("github_signin").await?;
    auth::start_device_flow().await
}

/// Poll the Device Flow until the user has typed the code or the
/// device-code has expired. Returns a discriminated union the
/// frontend switches on (Pending / SlowDown / Success / Error).
#[tauri::command]
pub async fn github_signin_poll(
    device_code: String,
    state: State<'_, AppState>,
) -> Result<PollResultDto, AppError> {
    state.require_network("github_signin").await?;
    let result: PollResult = auth::poll_device_flow(&device_code).await?;
    Ok(result.into())
}

/// Sign out: delete the token + scopes from the Keychain. No network.
/// We don't gate on paranoid mode (it's a *reduction* of state, never
/// an outbound call).
#[tauri::command]
pub async fn github_signout(_state: State<'_, AppState>) -> Result<(), AppError> {
    auth::signout()
}
