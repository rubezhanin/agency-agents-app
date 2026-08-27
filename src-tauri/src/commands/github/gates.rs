//! `authed_gate` — the 5-step pre-flight every Phase 12f authed action
//! runs before any network call.
//!
//! 1. Paranoid-mode gate. `state.require_network(feature)` — the
//!    single chokepoint the "Block all outbound" master switch flips.
//! 2. URL allowlist. `parse_github_url(homepage)` — strict
//!    `github.com/<owner>/<repo>`. Mismatch → `InvalidArgument`.
//! 3. Auth gate. `auth::read_token()` must return `Some(Token)` from
//!    the Keychain. None → `AuthRequired`.
//! 4. Scope gate. `auth::read_scopes()` must contain
//!    `required_scope`. Missing → `ScopeRequired { scope }`.
//! 5. Build the client (cheap — reqwest pools connections; we don't
//!    share across calls because the auth gate has to be re-checked
//!    every time anyway).
//!
//! Returns a `(client, repo, token)` triple on success; surfaces the
//! typed error on any gate failure.

use reqwest::Client;
use tauri::State;

use crate::error::AppError;
use crate::github::{actions, auth, parse_github_url, GithubRepo, Token};
use crate::state::AppState;

pub async fn authed_gate(
    state: State<'_, AppState>,
    homepage: &str,
    feature: &'static str,
    required_scope: &str,
) -> Result<(Client, GithubRepo, Token), AppError> {
    // 1. Paranoid-mode gate.
    state.require_network(feature).await?;

    // 2. URL allowlist. Authed actions use `InvalidArgument` (rather
    //    than the `Ok(None)` collapse `github_repo_stats` uses) because
    //    we shouldn't get this far if the homepage wasn't already
    //    classified as a GitHub URL on the frontend; an unparseable
    //    homepage here is a real bug, not a "no stats" outcome.
    let repo = parse_github_url(homepage).ok_or_else(|| AppError::InvalidArgument {
        message: format!("not a github.com/<owner>/<repo> URL: {homepage}"),
    })?;

    // 3. Auth gate.
    let token = auth::read_token()?.ok_or(AppError::AuthRequired)?;

    // 4. Scope gate. The scope list is cached at sign-in and read from
    //    the Keychain — no extra GitHub round-trip required.
    let scopes = auth::read_scopes()?.unwrap_or_default();
    if !scopes.iter().any(|s| s == required_scope) {
        return Err(AppError::ScopeRequired {
            scope: required_scope.to_string(),
        });
    }

    // 5. Build the client.
    let client = actions::build_client()?;
    Ok((client, repo, token))
}
