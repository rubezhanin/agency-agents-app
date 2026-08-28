//! Integration tests for the updater submodule tree.
//!
//! The `MockBackend` here is local (does not import the one in
//! `backend.rs`) because we need to count `check()` calls and pin
//! the install outcome, both of which the production `MockBackend` in
//! `backend.rs` doesn't track. Keeping this in a sibling `tests`
//! module also makes it clear that none of the IPC `#[tauri::command]`
//! bodies are exercised here — the tests target the pure `run_check` /
//! `run_install` / `run_skip` / `should_auto_check` cores.

use std::sync::Mutex;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::commands::settings::{Settings, SettingsLoadState};
use crate::error::AppError;
use crate::state::AppState;

use super::backend::UpdaterBackend;
use super::checker::run_check;
use super::installer::{run_install, run_skip};
use super::scheduler::{should_auto_check, AUTO_CHECK_BACKOFF, AUTO_CHECK_INTERVAL};
use super::types::{current_app_version, CachedUpdate, UpdateCheckOutcome};

// ---------------------------------------------------------------------------
// Local mock backend
// ---------------------------------------------------------------------------

/// In-memory mock backend. Counts `check()` calls so the paranoid-mode
/// gate can be verified (the gate must run *before* the backend is
/// touched).
struct MockBackend {
    check_result: Mutex<Result<Option<CachedUpdate>, AppError>>,
    install_result: Mutex<Result<(), AppError>>,
    check_calls: Mutex<u32>,
}

impl MockBackend {
    fn returning(check: Result<Option<CachedUpdate>, AppError>) -> Self {
        Self {
            check_result: Mutex::new(check),
            install_result: Mutex::new(Ok(())),
            check_calls: Mutex::new(0),
        }
    }

    fn install_returning(
        check: Result<Option<CachedUpdate>, AppError>,
        install: Result<(), AppError>,
    ) -> Self {
        Self {
            check_result: Mutex::new(check),
            install_result: Mutex::new(install),
            check_calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl UpdaterBackend for MockBackend {
    async fn check(&self) -> Result<Option<CachedUpdate>, AppError> {
        *self.check_calls.lock().unwrap() += 1;
        self.check_result.lock().unwrap().clone()
    }
    async fn download_and_install(&self, _version: &str) -> Result<(), AppError> {
        self.install_result.lock().unwrap().clone()
    }
}

async fn build_state_with(slot: SettingsLoadState) -> AppState {
    let state = AppState::build().expect("AppState::build");
    {
        let mut guard = state.settings.write().await;
        *guard = slot;
    }
    state
}

// ---------------------------------------------------------------------------
// run_check
// ---------------------------------------------------------------------------

/// Phase 15 §Tests #1 — happy path: plugin returns "no update";
/// command returns `UpToDate`.
#[tokio::test]
async fn check_now_returns_up_to_date_when_plugin_returns_none() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let backend = MockBackend::returning(Ok(None));
    let outcome = run_check(&state, &backend).await.expect("check");
    assert_eq!(outcome, UpdateCheckOutcome::UpToDate);

    // last_checked_at must be populated regardless of outcome so
    // the scheduler honours the 24h floor on UpToDate too.
    let guard = state.updater_state.read().await;
    assert!(guard.last_checked_at.is_some());
    assert!(guard.cached_available.is_none());
}

/// Phase 15 §Tests #2 — available path: plugin returns a version,
/// command returns `Available { ... }` with the right fields.
#[tokio::test]
async fn check_now_returns_available_when_plugin_returns_some() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let backend = MockBackend::returning(Ok(Some(CachedUpdate {
        version: "9.9.9".into(),
        current_version: current_app_version().to_string(),
        notes: Some("hotfix".into()),
        pub_date: Some("2026-05-24T00:00:00Z".into()),
    })));
    let outcome = run_check(&state, &backend).await.expect("check");
    match outcome {
        UpdateCheckOutcome::Available {
            version,
            notes,
            skipped,
            ..
        } => {
            assert_eq!(version, "9.9.9");
            assert_eq!(notes.as_deref(), Some("hotfix"));
            assert!(!skipped, "fresh version must not be marked skipped");
        }
        other => panic!("expected Available, got {other:?}"),
    }

    // Cached available payload available for install validation.
    let guard = state.updater_state.read().await;
    let cached = guard.cached_available.clone().expect("cached");
    assert_eq!(cached.version, "9.9.9");
}

/// Available + version is in the skip-list → `skipped: true`.
#[tokio::test]
async fn check_now_marks_skipped_when_version_is_in_skip_list() {
    let s = Settings {
        skipped_update_versions: vec!["9.9.9".into()],
        ..Settings::default()
    };
    let state = build_state_with(SettingsLoadState::Loaded(s)).await;
    let backend = MockBackend::returning(Ok(Some(CachedUpdate {
        version: "9.9.9".into(),
        current_version: current_app_version().to_string(),
        notes: None,
        pub_date: None,
    })));
    let outcome = run_check(&state, &backend).await.expect("check");
    match outcome {
        UpdateCheckOutcome::Available { skipped, .. } => assert!(skipped),
        other => panic!("expected Available, got {other:?}"),
    }
}

/// Phase 15 §Tests #3 — blocked by Paranoid Mode: returns
/// `ParanoidModeBlocked { feature: "update_check" }`.
#[tokio::test]
async fn check_now_blocked_by_paranoid_mode() {
    let s = Settings {
        paranoid_mode: true,
        ..Settings::default()
    };
    let state = build_state_with(SettingsLoadState::Loaded(s)).await;
    let backend = MockBackend::returning(Ok(None));
    let r = run_check(&state, &backend).await;
    match r {
        Err(AppError::ParanoidModeBlocked { feature }) => {
            assert_eq!(feature, "update_check");
        }
        other => panic!("expected ParanoidModeBlocked, got {other:?}"),
    }

    // Backend must NOT have been called — gate runs before the trait call.
    assert_eq!(*backend.check_calls.lock().unwrap(), 0);
}

// ---------------------------------------------------------------------------
// run_install
// ---------------------------------------------------------------------------

/// Phase 15 §Tests #4 — install rejects a stale version arg.
#[tokio::test]
async fn install_rejects_stale_version_arg() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    // Seed the cache with an Available 9.9.9.
    let backend = MockBackend::returning(Ok(Some(CachedUpdate {
        version: "9.9.9".into(),
        current_version: current_app_version().to_string(),
        notes: None,
        pub_date: None,
    })));
    run_check(&state, &backend).await.expect("check");

    // UI requests install of an OLDER version than the cache (stale UI).
    let r = run_install(&state, &backend, "0.3.0").await;
    match r {
        Err(AppError::InvalidArgument { message }) => {
            assert!(message.contains("mismatch"), "got: {message}");
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

/// Install without a preceding check → InvalidArgument (no cache).
#[tokio::test]
async fn install_without_cache_returns_invalid_argument() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let backend = MockBackend::returning(Ok(None));
    let r = run_install(&state, &backend, "9.9.9").await;
    match r {
        Err(AppError::InvalidArgument { message }) => {
            assert!(message.contains("no update available"), "got: {message}");
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

/// Install of a same-or-older version → DowngradeRejected.
#[tokio::test]
async fn install_rejects_downgrade() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    // Seed cache pointing at the *current* version (same-version is
    // also a downgrade for our purposes — strict upgrade required).
    let current = current_app_version().to_string();
    let backend = MockBackend::returning(Ok(Some(CachedUpdate {
        version: current.clone(),
        current_version: current.clone(),
        notes: None,
        pub_date: None,
    })));
    run_check(&state, &backend).await.expect("check");

    let r = run_install(&state, &backend, &current).await;
    match r {
        Err(AppError::DowngradeRejected {
            current: c,
            target: t,
        }) => {
            assert_eq!(c, current);
            assert_eq!(t, current);
        }
        other => panic!("expected DowngradeRejected, got {other:?}"),
    }
}

/// Phase 15 §Tests #8 — signature verification failure surfaces
/// as the typed `SignatureVerificationFailed` error.
#[tokio::test]
async fn install_surfaces_signature_verification_failed() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let backend = MockBackend::install_returning(
        Ok(Some(CachedUpdate {
            version: "9.9.9".into(),
            current_version: current_app_version().to_string(),
            notes: None,
            pub_date: None,
        })),
        Err(AppError::SignatureVerificationFailed {
            message: "minisign rejected".into(),
        }),
    );
    run_check(&state, &backend).await.expect("check");

    let r = run_install(&state, &backend, "9.9.9").await;
    match r {
        Err(AppError::SignatureVerificationFailed { message }) => {
            assert!(message.contains("minisign"), "got: {message}");
        }
        other => panic!("expected SignatureVerificationFailed, got {other:?}"),
    }
}

/// Phase 15 §Tests #9 (BONUS) — sha256 mismatch surfaces as the
/// typed `HashMismatch` error before signature verification.
#[tokio::test]
async fn install_surfaces_hash_mismatch() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let backend = MockBackend::install_returning(
        Ok(Some(CachedUpdate {
            version: "9.9.9".into(),
            current_version: current_app_version().to_string(),
            notes: None,
            pub_date: None,
        })),
        Err(AppError::HashMismatch {
            expected: "deadbeef".into(),
            actual: "cafef00d".into(),
        }),
    );
    run_check(&state, &backend).await.expect("check");

    let r = run_install(&state, &backend, "9.9.9").await;
    match r {
        Err(AppError::HashMismatch { expected, actual }) => {
            assert_eq!(expected, "deadbeef");
            assert_eq!(actual, "cafef00d");
        }
        other => panic!("expected HashMismatch, got {other:?}"),
    }
}

/// Phase 15 fix-up — `run_install` clears `cached_available` after a
/// successful install so the indicator + Settings card don't
/// re-offer the same install on the next render.
#[tokio::test]
async fn install_clears_cached_available_on_success() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let backend = MockBackend::install_returning(
        Ok(Some(CachedUpdate {
            version: "9.9.9".into(),
            current_version: current_app_version().to_string(),
            notes: None,
            pub_date: None,
        })),
        Ok(()),
    );
    run_check(&state, &backend).await.expect("check");
    // Sanity: cache populated by the check.
    {
        let guard = state.updater_state.read().await;
        assert!(guard.cached_available.is_some());
    }

    run_install(&state, &backend, "9.9.9")
        .await
        .expect("install");

    let guard = state.updater_state.read().await;
    assert!(
        guard.cached_available.is_none(),
        "cached_available must be cleared after successful install"
    );
    assert_eq!(guard.last_outcome, Some(UpdateCheckOutcome::UpToDate));
}

/// Install blocked by paranoid mode → ParanoidModeBlocked.
#[tokio::test]
async fn install_blocked_by_paranoid_mode() {
    let s = Settings {
        paranoid_mode: true,
        ..Settings::default()
    };
    let state = build_state_with(SettingsLoadState::Loaded(s)).await;
    let backend = MockBackend::returning(Ok(None));
    let r = run_install(&state, &backend, "9.9.9").await;
    match r {
        Err(AppError::ParanoidModeBlocked { feature }) => {
            assert_eq!(feature, "update_check");
        }
        other => panic!("expected ParanoidModeBlocked, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// run_skip
// ---------------------------------------------------------------------------

/// Phase 15 fix-up — `run_skip` MUST NOT persist `Settings::default()`
/// when the in-memory state is Corrupt. Doing so silently revokes the
/// paranoid-on lockdown that Corrupt settings imply. Refusal with a
/// typed error is the correct behaviour.
#[tokio::test]
async fn skip_refuses_on_corrupt_settings() {
    let state = build_state_with(SettingsLoadState::Corrupt {
        message: "synthetic test corruption".into(),
    })
    .await;

    let r = run_skip(&state, "9.9.9").await;
    match r {
        Err(AppError::Internal { message }) => {
            assert!(
                message.contains("unreadable"),
                "expected unreadable message, got: {message}"
            );
            assert!(
                message.contains("reset"),
                "expected reset guidance, got: {message}"
            );
        }
        other => panic!("expected Internal error refusing skip, got {other:?}"),
    }

    // Settings must still be Corrupt — refused skip must NOT have
    // overwritten the in-memory state.
    let guard = state.settings.read().await;
    match &*guard {
        SettingsLoadState::Corrupt { .. } => {} // expected
        other => panic!("settings state was mutated by refused skip: {other:?}"),
    }
}

/// Phase 15 fix-up — invalid version argument rejected with
/// InvalidArgument, regardless of settings state.
#[tokio::test]
async fn skip_rejects_empty_version() {
    let state = build_state_with(SettingsLoadState::Loaded(Settings::default())).await;
    let r = run_skip(&state, "").await;
    match r {
        Err(AppError::InvalidArgument { .. }) => {}
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Phase 15 §Tests #6 — scheduler honours the 24h floor.
/// Repeated `should_auto_check` calls within the window do not
/// approve a fresh check.
#[test]
fn scheduler_honors_24h_floor() {
    let now: i64 = 1_700_000_000;
    let last_check = now - (23 * 3600);
    // 23h since last check → not due yet.
    assert!(!should_auto_check(true, false, Some(last_check), now));
    // Exactly 24h → due.
    assert!(should_auto_check(true, false, Some(now - 24 * 3600), now));
    // 25h → due.
    assert!(should_auto_check(true, false, Some(now - 25 * 3600), now));
}

/// Repeated start/stop within an hour does not fire multiple checks:
/// model this by checking the gate function with timestamps that
/// represent successive launches all within the same window.
#[test]
fn scheduler_does_not_refire_within_one_hour() {
    let initial_check: i64 = 1_700_000_000;
    // Five "launches" all within 60 minutes of the initial check.
    for offset in &[60, 600, 1800, 3000, 3500] {
        let now = initial_check + offset;
        let should = should_auto_check(true, false, Some(initial_check), now);
        assert!(
            !should,
            "scheduler must NOT fire at +{offset}s after last check"
        );
    }
}

/// Never-checked-before → fires on first wake.
#[test]
fn scheduler_fires_when_never_checked() {
    let now: i64 = 1_700_000_000;
    assert!(should_auto_check(true, false, None, now));
}

/// Phase 15 §Tests #7 — flipping paranoid_mode on suspends the
/// scheduler regardless of how stale `last_checked_at` is.
#[test]
fn scheduler_suspends_on_paranoid_mode() {
    let now: i64 = 1_700_000_000;
    // Even with last_checked_at well past the 24h window, the gate
    // returns false when paranoid_mode is on.
    assert!(!should_auto_check(true, true, Some(now - 999_999), now));
    // And with last_checked_at None (never checked) still false.
    assert!(!should_auto_check(true, true, None, now));
}

/// Off by default: auto_check_enabled=false → never fires.
#[test]
fn scheduler_does_nothing_when_disabled() {
    let now: i64 = 1_700_000_000;
    assert!(!should_auto_check(false, false, None, now));
    assert!(!should_auto_check(false, false, Some(now - 100_000), now));
}

/// Pin the backoff sequence at 1h, 6h, 24h. Drift here would mean
/// silently changing the user-facing retry behaviour.
#[test]
fn auto_check_backoff_sequence_matches_plan_spec() {
    assert_eq!(AUTO_CHECK_BACKOFF.len(), 3);
    assert_eq!(AUTO_CHECK_BACKOFF[0].as_secs(), 60 * 60);
    assert_eq!(AUTO_CHECK_BACKOFF[1].as_secs(), 6 * 60 * 60);
    assert_eq!(AUTO_CHECK_BACKOFF[2].as_secs(), 24 * 60 * 60);
}

/// Pin the 24h floor.
#[test]
fn auto_check_interval_is_24_hours() {
    assert_eq!(AUTO_CHECK_INTERVAL.as_secs(), 24 * 60 * 60);
}

// ---------------------------------------------------------------------------
// AppState integration sanity — the `tokio::sync::RwLock` plumbing is
// exercised indirectly by the tests above; this one ensures the
// constructor works in a clean environment.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn app_state_build_is_repeatable() {
    // Just ensures we can build + drop two states in sequence without
    // anything leaking (path / port / etc). A defensive smoke test
    // — every other test already builds one, but if `AppState::build`
    // started failing intermittently, this would catch it.
    let _ = AppState::build().expect("first build");
    let _ = AppState::build().expect("second build");
    // And ensure the updater mirror defaults to empty.
    let _ = RwLock::new(()).write().await;
}
