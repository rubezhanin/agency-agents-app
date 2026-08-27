//! Wire types + semver helpers for the in-app updater.
//!
//! Pure data — no IPC, no Tauri state, no async. Imported by every other
//! module in `commands::updater::*`. Kept separate so the schema is
//! reviewable in one place.

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Outcome of [`super::checker::update_check_now`]. Mirrors the three real
/// states the plugin can return, flattened into a single discriminated
/// union the frontend can `switch` over.
///
/// The `Blocked` variant is **not** returned by this enum — paranoid
/// mode surfaces as `Err(AppError::ParanoidModeBlocked)` instead, so
/// the toast routes through the same channel as every other gated call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UpdateCheckOutcome {
    /// Plugin returned no update — running version is current.
    UpToDate,
    /// Plugin returned an update. Fields the UI needs to render the
    /// indicator pill + Settings card.
    Available {
        /// Announced version (semver) — used by `update_install` as
        /// the sanity-check arg.
        version: String,
        /// Currently-installed version, surfaced so the UI can render
        /// "v0.3.0 → v0.3.1" without an extra IPC call.
        current_version: String,
        /// Release-notes body from the manifest. Optional.
        #[serde(skip_serializing_if = "Option::is_none")]
        notes: Option<String>,
        /// Publish date (RFC 3339), if present in the manifest.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub_date: Option<String>,
        /// True iff the user has already skipped this version via the
        /// title-bar indicator's `x`. UI uses this to suppress the
        /// re-display of the indicator (Settings panel still shows the
        /// card so the user can install if they change their mind).
        skipped: bool,
    },
}

impl UpdateCheckOutcome {
    /// Wire shape tag — used by the frontend to discriminate variants
    /// without `instanceof`. Mirrors the `serde(rename_all = "camelCase")`
    /// on the enum.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UpToDate => "upToDate",
            Self::Available { .. } => "available",
        }
    }
}

/// Subset of plugin Update fields we cache for the `update_install`
/// stale-version sanity check + the auto-check scheduler's "last
/// available" state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedUpdate {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
}

/// In-memory mirror of the latest update check result. Stored on
/// `AppState.updater_state` so the auto-check scheduler and the
/// `update_install` validator share the same view.
#[derive(Debug, Default)]
pub struct UpdaterState {
    /// Latest plugin outcome, if a check has run.
    ///
    /// `#[allow(dead_code)]` because in `cfg(test)` the
    /// scheduler's `read_scheduler_inputs` is gated out and the IPC
    /// body is a stub; the field is written by `run_check` (also
    /// gated). Real consumers live in `state::AppState` and the
    /// title-bar indicator.
    #[allow(dead_code)]
    pub last_outcome: Option<UpdateCheckOutcome>,
    /// Unix timestamp (seconds) of the most recent successful check.
    /// Used by the scheduler's 24h-floor enforcement so cross-launch
    /// behaviour is predictable.
    #[allow(dead_code)]
    pub last_checked_at: Option<i64>,
    /// Cached `Available` payload for the install-arg sanity check.
    /// Cleared when the outcome flips back to `UpToDate`.
    pub cached_available: Option<CachedUpdate>,
}

/// Currently-running app version, surfaced for downgrade rejection
/// without bringing the plugin into the public API. Resolved once at
/// startup from `CARGO_PKG_VERSION`.
pub fn current_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compare two semver strings; returns `true` when `target` is greater
/// than `current`. Used by the explicit downgrade-rejection check in
/// `update_install`. Falls back to lexicographic comparison if either
/// string fails semver parsing (defensive — the plugin's own version
/// comparator handles the normal case, this is a final-line check).
pub fn is_strict_upgrade(current: &str, target: &str) -> bool {
    // Strip a leading `v` if present so "v0.3.1" parses cleanly.
    let trim = |s: &str| s.trim_start_matches('v').to_string();
    let cur = trim(current);
    let tgt = trim(target);
    match (parse_semver(&cur), parse_semver(&tgt)) {
        (Some(c), Some(t)) => t > c,
        // Any unparseable input: refuse the upgrade. Safer to fall back
        // to "manual install" than to ship the wrong binary.
        _ => false,
    }
}

/// Minimal three-tuple semver parser. We don't need pre-release /
/// metadata handling for our own release cadence (numeric major.minor.patch
/// only); a full semver crate is overkill.
pub(crate) fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let mut iter = s.splitn(3, '.');
    let major: u32 = iter.next()?.parse().ok()?;
    let minor: u32 = iter.next()?.parse().ok()?;
    let patch_raw = iter.next()?;
    // Drop pre-release / metadata suffix (`0.3.1-beta.1`).
    let patch_str = patch_raw.split(['-', '+']).next().unwrap_or(patch_raw);
    let patch: u32 = patch_str.parse().ok()?;
    Some((major, minor, patch))
}

// ===========================================================================
// Tests — moved verbatim from `commands/updater.rs` so the refactor is
// safe-by-default: the assertions match the previous behaviour exactly.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semver_round_trips_basic() {
        assert_eq!(parse_semver("0.3.1"), Some((0, 3, 1)));
        assert_eq!(parse_semver("1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_semver("12.345.6789"), Some((12, 345, 6789)));
    }

    #[test]
    fn parse_semver_strips_prerelease() {
        assert_eq!(parse_semver("0.3.1-beta.1"), Some((0, 3, 1)));
        assert_eq!(parse_semver("0.3.1+build.7"), Some((0, 3, 1)));
    }

    #[test]
    fn parse_semver_rejects_garbage() {
        assert_eq!(parse_semver("not a version"), None);
        assert_eq!(parse_semver("1.2"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn is_strict_upgrade_basic() {
        assert!(is_strict_upgrade("0.3.0", "0.3.1"));
        assert!(is_strict_upgrade("0.3.0", "0.4.0"));
        assert!(is_strict_upgrade("0.3.0", "1.0.0"));
    }

    #[test]
    fn is_strict_upgrade_rejects_same_or_older() {
        assert!(!is_strict_upgrade("0.3.0", "0.3.0"));
        assert!(!is_strict_upgrade("0.3.1", "0.3.0"));
        assert!(!is_strict_upgrade("1.0.0", "0.99.99"));
    }

    #[test]
    fn is_strict_upgrade_handles_v_prefix() {
        assert!(is_strict_upgrade("v0.3.0", "v0.3.1"));
        assert!(is_strict_upgrade("0.3.0", "v0.3.1"));
    }

    #[test]
    fn is_strict_upgrade_unparseable_rejects() {
        assert!(!is_strict_upgrade("garbage", "0.3.1"));
        assert!(!is_strict_upgrade("0.3.0", "also garbage"));
    }

    /// `UpdateCheckOutcome` serializes with the `kind` tag in camelCase
    /// and `Available` carries the fields the frontend expects.
    #[test]
    fn update_check_outcome_wire_shape() {
        let v = serde_json::to_value(UpdateCheckOutcome::UpToDate).unwrap();
        assert_eq!(v["kind"], "upToDate");

        let v = serde_json::to_value(UpdateCheckOutcome::Available {
            version: "0.3.1".into(),
            current_version: "0.3.0".into(),
            notes: Some("changelog".into()),
            pub_date: Some("2026-05-24T00:00:00Z".into()),
            skipped: false,
        })
        .unwrap();
        assert_eq!(v["kind"], "available");
        assert_eq!(v["version"], "0.3.1");
        assert_eq!(v["currentVersion"], "0.3.0");
        assert_eq!(v["notes"], "changelog");
        assert_eq!(v["pubDate"], "2026-05-24T00:00:00Z");
        assert_eq!(v["skipped"], false);
    }

    /// `current_app_version` returns a parseable semver-shaped string.
    #[test]
    fn current_app_version_is_semver_shaped() {
        let v = current_app_version();
        assert!(parse_semver(v).is_some(), "{v} did not parse as semver");
    }

    /// The `kind()` helper agrees with the serialised `kind` tag, so
    /// the frontend can either `switch` on it or use the helper.
    #[test]
    fn kind_helper_matches_serialised_tag() {
        assert_eq!(UpdateCheckOutcome::UpToDate.kind(), "upToDate");
        let v = UpdateCheckOutcome::Available {
            version: "0.3.1".into(),
            current_version: "0.3.0".into(),
            notes: None,
            pub_date: None,
            skipped: false,
        };
        assert_eq!(v.kind(), "available");
    }

    // -- harness used by the other updater submodules' tests --------------
    // (re-exported here so the refactor doesn't need a public re-export
    // for tests that need a minimal AppState).

    /// Minimal `AppError` factory used by tests in the other submodules.
    /// Kept private to the tests so it doesn't leak into production paths.
    #[allow(dead_code)]
    pub(crate) fn _test_error_marker() -> AppError {
        AppError::Internal {
            message: "test".into(),
        }
    }
}
