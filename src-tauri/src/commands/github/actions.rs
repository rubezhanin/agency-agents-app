//! Authed-action IPC: star / unstar / is_starred / watch / unwatch /
//! create_issue.
//!
//! Each of these funnels through `gates::authed_gate`, which applies
//! the 5-step pre-flight (paranoid, URL allowlist, auth, scope,
//! client) before any of them talks to the network. The actual
//! `reqwest` call lives in `crate::github::actions`; this module is
//! the thin IPC layer that wires the gate to the action.

use tauri::State;

use crate::error::AppError;
use crate::github::CreatedIssue;
use crate::state::AppState;

use super::gates::authed_gate;
use super::types::{SCOPE_NOTIFICATIONS, SCOPE_PUBLIC_REPO};

/// Star a repo. Requires `public_repo` scope.
#[tauri::command]
pub async fn github_star(homepage: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let (client, repo, token) =
        authed_gate(state, &homepage, "github_star", SCOPE_PUBLIC_REPO).await?;
    crate::github::actions::star(&client, &repo, &token).await
}

/// Unstar a repo. Requires `public_repo` scope.
#[tauri::command]
pub async fn github_unstar(homepage: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let (client, repo, token) =
        authed_gate(state, &homepage, "github_unstar", SCOPE_PUBLIC_REPO).await?;
    crate::github::actions::unstar(&client, &repo, &token).await
}

/// Read whether the signed-in user has starred a repo. Requires
/// `public_repo` scope.
#[tauri::command]
pub async fn github_is_starred(
    homepage: String,
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    let (client, repo, token) =
        authed_gate(state, &homepage, "github_is_starred", SCOPE_PUBLIC_REPO).await?;
    crate::github::actions::is_starred(&client, &repo, &token).await
}

/// Watch a repo (subscribes the user to release notifications).
/// Requires `notifications` scope (NOT implied by `public_repo`).
#[tauri::command]
pub async fn github_watch(homepage: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let (client, repo, token) =
        authed_gate(state, &homepage, "github_watch", SCOPE_NOTIFICATIONS).await?;
    crate::github::actions::watch(&client, &repo, &token).await
}

/// Unwatch a repo. Requires `notifications` scope.
#[tauri::command]
pub async fn github_unwatch(homepage: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let (client, repo, token) =
        authed_gate(state, &homepage, "github_unwatch", SCOPE_NOTIFICATIONS).await?;
    crate::github::actions::unwatch(&client, &repo, &token).await
}

/// File an issue on a repo. `title` and `body` are sanitised by the
/// actions layer (length cap, control-char strip, body ≤ 64 KiB).
/// `labels` are the labels to attach, sanitised against GitHub's
/// label-naming rules (≤ 50 chars, no reserved chars, ≤ 10 labels).
/// Requires `public_repo` scope.
#[tauri::command]
pub async fn github_create_issue(
    homepage: String,
    title: String,
    body: String,
    labels: Vec<String>,
    state: State<'_, AppState>,
) -> Result<CreatedIssue, AppError> {
    let (client, repo, token) =
        authed_gate(state, &homepage, "github_create_issue", SCOPE_PUBLIC_REPO).await?;
    // Convert Vec<String> to &[&str] for the borrowed-slice API. The
    // sanitiser then takes owned Strings back for the JSON payload.
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    crate::github::actions::create_issue(&client, &repo, &token, &title, &body, &label_refs).await
}
