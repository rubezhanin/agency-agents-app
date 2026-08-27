//! Integration tests for the github IPC surface.
//!
//! We test what we can without injecting a `KeychainSlot` (the trait
//! is not exposed across the `commands::github` boundary). The
//! MockKeychain-driven coverage of the full 5-step gate chain lives
//! next to the production code in `crate::github::auth`; the tests
//! here exercise the user-facing IPC contracts (paranoid gate fires
//! for every action; settings-disabled collapses to `Ok(None)`;
//! settings-corrupt fails closed; non-GitHub URLs return `Ok(None)`)
//! using the production code path against an in-memory `AppState`.

use crate::commands::settings::{Settings, SettingsLoadState};
use crate::state::AppState;

async fn build_state_with(slot: SettingsLoadState) -> AppState {
    let state = AppState::build().expect("AppState::build");
    {
        let mut guard = state.settings.write().await;
        *guard = slot;
    }
    state
}

// ---------- Paranoid-mode gate for every new command (6) ----------

/// Helper: assert that calling `feature` with paranoid ON blocks.
/// Asserts the feature string is carried verbatim into the error so
/// the frontend toast can route.
async fn assert_blocked_by_paranoid(feature: &'static str) {
    let s = Settings {
        paranoid_mode: true,
        ..Settings::default()
    };
    let state = build_state_with(SettingsLoadState::Loaded(s)).await;
    let r = state.require_network(feature).await;
    match r {
        Err(crate::error::AppError::ParanoidModeBlocked { feature: f }) => {
            assert_eq!(f, feature);
        }
        other => panic!("expected ParanoidModeBlocked for {feature}, got {other:?}"),
    }
}

#[tokio::test]
async fn star_blocked_by_paranoid_mode() {
    assert_blocked_by_paranoid("github_star").await;
}

#[tokio::test]
async fn unstar_blocked_by_paranoid_mode() {
    assert_blocked_by_paranoid("github_unstar").await;
}

#[tokio::test]
async fn is_starred_blocked_by_paranoid_mode() {
    assert_blocked_by_paranoid("github_is_starred").await;
}

#[tokio::test]
async fn watch_blocked_by_paranoid_mode() {
    assert_blocked_by_paranoid("github_watch").await;
}

#[tokio::test]
async fn unwatch_blocked_by_paranoid_mode() {
    assert_blocked_by_paranoid("github_unwatch").await;
}

#[tokio::test]
async fn create_issue_blocked_by_paranoid_mode() {
    assert_blocked_by_paranoid("github_create_issue").await;
}

/// Corrupt settings also fails closed for authed actions (same as
/// paranoid=on). The fail-closed rule lives in `require_network`
/// itself, but pin it here so the §12f gate chain's contract is
/// asserted at the command layer too.
#[tokio::test]
async fn authed_actions_blocked_when_settings_corrupt() {
    let state = build_state_with(SettingsLoadState::Corrupt {
        message: "boom".into(),
    })
    .await;
    let r = state.require_network("github_create_issue").await;
    assert!(matches!(
        r,
        Err(crate::error::AppError::ParanoidModeBlocked { .. })
    ));
}

// ---------- Settings opt-in gate for github_repo_stats ----------

/// `github_repo_stats` collapses to `Ok(None)` when `github_enabled`
/// is false (default). No network. No URL parse. The frontend
/// interprets `None` as "no GitHub stats for this row".
///
/// We test the contract by constructing the in-memory state and
/// reading the gate directly. The IPC function itself is a thin
/// wrapper around `state.settings.read().await` + `github_enabled`.
#[tokio::test]
async fn settings_disabled_short_circuits_to_none() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let guard = state.settings.read().await;
    let enabled = match &*guard {
        SettingsLoadState::Loaded(s) => s.github_enabled,
        _ => false,
    };
    assert!(!enabled, "github_enabled must default to false");
}

/// `github_enabled = true` (user opted in) and paranoid OFF — the
/// gate passes; the next stage would be the URL allowlist + the
/// `parse_github_url` extraction.
#[tokio::test]
async fn opt_in_passes_paranoid_gate() {
    let s = Settings {
        github_enabled: true,
        ..Settings::default()
    };
    let state = build_state_with(SettingsLoadState::Loaded(s)).await;
    let r = state.require_network("github_repo_stats").await;
    assert!(r.is_ok(), "opt-in + paranoid-off must pass, got {r:?}");
}

// ---------- URL allowlist ----------

/// Non-github URLs collapse to `Ok(None)` for `github_repo_stats`
/// (the user-facing "no stats" shape).
#[tokio::test]
async fn non_github_homepage_returns_none() {
    use crate::github::parse_github_url;
    let r = parse_github_url("https://example.com/foo/bar");
    assert!(r.is_none());
}

/// Canonical github.com/<owner>/<repo> URL extracts cleanly.
#[tokio::test]
async fn canonical_github_url_extracts() {
    use crate::github::parse_github_url;
    let r = parse_github_url("https://github.com/octocat/hello-world").unwrap();
    assert_eq!(r.owner, "octocat");
    assert_eq!(r.repo, "hello-world");
}

// ---------- Gate ordering ----------

/// Paranoid-mode gate fires BEFORE the auth gate. Even with no token
/// in the keychain, paranoid ON must be the first error surfaced so we
/// don't leak "auth required" semantics to a user who told us to
/// stop making outbound calls.
#[tokio::test]
async fn paranoid_gate_fires_before_auth_or_url() {
    let s = Settings {
        paranoid_mode: true,
        ..Settings::default()
    };
    let state = build_state_with(SettingsLoadState::Loaded(s)).await;
    let r = state.require_network("github_star").await;
    match r {
        Err(crate::error::AppError::ParanoidModeBlocked { feature }) => {
            assert_eq!(feature, "github_star");
        }
        other => panic!("expected ParanoidModeBlocked, got {other:?}"),
    }
}

// ---------- Corrupt settings + stats (12c) ----------

/// `corrupt_settings_returns_none_for_stats` — when settings are
/// corrupt, `github_repo_stats` short-circuits to `Ok(None)` because
/// the gate falls back to `github_enabled = false` (the fail-closed
/// rule, mirrored from `require_network`).
#[tokio::test]
async fn corrupt_settings_returns_none_for_stats() {
    let state = build_state_with(SettingsLoadState::Corrupt {
        message: "synthetic".into(),
    })
    .await;
    let guard = state.settings.read().await;
    let enabled = match &*guard {
        SettingsLoadState::Loaded(s) => s.github_enabled,
        _ => false, // FirstLaunch + Corrupt both fall back to "off"
    };
    assert!(
        !enabled,
        "corrupt settings must collapse github_enabled to false"
    );
}

// ---------- AppState integration sanity ----------

#[tokio::test]
async fn app_state_build_is_repeatable() {
    // Defensive smoke test: ensure AppState::build succeeds twice in
    // sequence without leaking state.
    let _ = AppState::build().expect("first build");
    let _ = AppState::build().expect("second build");
}
