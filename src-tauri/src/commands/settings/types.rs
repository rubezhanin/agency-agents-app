//! Settings wire types + `clamp` / `push_skipped_version` helpers.
//!
//! Pure data — no IO, no Tauri state, no async. Imported by every
//! other module in `commands::settings::*`. Kept separate so the
//! schema is reviewable in one place and tests can exercise the
//! `clamp` and skip-list rules in isolation.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Hard cap on settings.json size. 1 MiB is wildly generous for what is
/// at most a few dozen scalar fields — protects against accidental or
/// hostile bloat (e.g. a future bug that appends to an array forever).
pub const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

/// On-disk + IPC payload. Every field has `#[serde(default)]` so a
/// future version that adds a field reads cleanly into an older shape
/// (missing fields take their defaults) and an older version reading a
/// newer file ignores fields it doesn't know about.
///
/// **Numeric clamping** is applied by [`Self::clamp`] after every load
/// and before every save. Don't bypass it — the caps are part of the
/// contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Master "block all outbound network" switch. When true,
    /// `require_network` denies every call. Default false (first launch
    /// = current behaviour preserved).
    pub paranoid_mode: bool,

    /// Show the "Catalog is N days old — refresh?" banner when the
    /// active catalog is at least this many days old. Default 14.
    /// Clamped to `[1, 365]` on every load and save.
    pub catalog_stale_banner_days: u32,

    /// Legacy icon-fetching mode inherited from the source app. Retained in
    /// the settings schema for compatibility until the network settings model
    /// is pruned.
    pub cask_icon_mode: CaskIconMode,

    /// Trending cache TTL in minutes. Default 60 (matches the existing
    /// `TRENDING_TTL` in `trending/cache.rs`). Clamped to `[5, 1440]`
    /// on every load and save — five minutes minimum to be a polite
    /// client, 24 hours maximum because anything older would be stale.
    pub trending_ttl_minutes: u32,

    /// Phase 12c — when true, PackageDetail probes `api.github.com` for
    /// repo stats whenever the package's homepage is a GitHub URL.
    /// Default **false** (off) so the v0.1.x posture of "no GitHub
    /// traffic unless the user opts in" is preserved on every fresh
    /// install. The runtime gate is `commands::github::*` which
    /// short-circuits to `Ok(None)` when this is false — before any
    /// outbound call. Paranoid mode overrides this regardless.
    pub github_enabled: bool,

    /// Phase 13 — master AI Features toggle. When false, AI-derived
    /// presentation data is hidden in the UI. Default **true**.
    ///
    /// This is a *rendering* gate — the enrichment payload is bundled
    /// into the binary regardless, so toggling this on/off doesn't
    /// trigger any I/O, network, or LLM calls.
    #[serde(default = "default_ai_features_enabled")]
    pub ai_features_enabled: bool,

    /// Phase 15 — opt-in daily auto-check for in-app updates. Default
    /// **false** so a fresh install never reaches out to the manifest
    /// endpoint without the user clicking either the manual "Check for
    /// updates" button or this toggle. When enabled (and Offline Mode
    /// is off), the scheduler in [`crate::commands::updater`] wakes
    /// every 24 h and runs `update_check_now`. Paranoid mode and a
    /// `Corrupt` settings state both suppress the scheduler — same gate
    /// every other outbound feature consults.
    #[serde(default)]
    pub update_auto_check: bool,

    /// Phase 15 — versions the user explicitly dismissed via the
    /// title-bar indicator's `×` button. Bounded at 10 entries with
    /// oldest-evicted-on-push (see [`Settings::push_skipped_version`]).
    /// The skip is per-version: a *newer* release re-triggers the
    /// indicator even if every previous version is in this list.
    #[serde(default)]
    pub skipped_update_versions: Vec<String>,

    /// Legacy enhanced-trending toggle inherited from the source app.
    /// Retained for settings-file compatibility; Agency Agents should not
    /// wire a runtime feature to this without a fresh endpoint audit.
    #[serde(default)]
    pub enhanced_trending_enabled: bool,

    /// Legacy vulnerability-scanning toggle inherited from the source app.
    /// Retained for settings-file compatibility; Agency Agents does not shell
    /// out to a vulnerability scanner.
    #[serde(default)]
    pub vulnerability_scanning_enabled: bool,

    /// Legacy live-enrichment toggle inherited from the source app. Retained
    /// for settings-file compatibility; Agency Agents currently reads metadata
    /// from the active AA catalog.
    #[serde(default)]
    pub live_enrichment_enabled: bool,

    /// Per-tool custom install base path (tool id → absolute base directory).
    /// When set for a tool, user-scope installs + detection resolve against
    /// this base instead of the OS home — e.g. pointing Claude Code at a WSL
    /// home (`\\wsl.localhost\Ubuntu\home\me`) from the Windows app. An empty
    /// or absent entry means "use the OS home". Project-scope installs are
    /// unaffected (they resolve against the chosen project root).
    #[serde(default)]
    pub tool_paths: HashMap<String, String>,
}

/// Default factory for [`Settings::ai_features_enabled`] — separated
/// out so `#[serde(default = "…")]` can pick it up for forward-compat
/// on settings.json files written before Phase 13.
fn default_ai_features_enabled() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            paranoid_mode: false,
            catalog_stale_banner_days: 14,
            cask_icon_mode: CaskIconMode::All,
            trending_ttl_minutes: 60,
            // Off by default per Phase 12c plan: anonymous GitHub probes
            // are opt-in so first-launch posture stays "zero outbound
            // beyond what the user has already consented to".
            github_enabled: false,
            // On by default per Phase 13 plan: AI-enriched rendering is
            // a value-add the project wants to show off out of the box.
            // Toggling off reverts the UI to plain source/catalog metadata.
            ai_features_enabled: default_ai_features_enabled(),
            // Off by default per Phase 15 plan: the manifest endpoint
            // stays cold until the user explicitly opts in (or hits the
            // manual "Check for updates" button).
            update_auto_check: false,
            // Empty by default — populated as the user dismisses
            // individual versions via the title-bar indicator's `×`.
            skipped_update_versions: Vec::new(),
            // Off by default; retained legacy field.
            enhanced_trending_enabled: false,
            // Off by default; retained legacy field.
            vulnerability_scanning_enabled: false,
            // Off by default; retained legacy field.
            live_enrichment_enabled: false,
            // Empty by default — user opts a tool into a custom base path
            // (e.g. a WSL home) from the Tools panel.
            tool_paths: HashMap::new(),
        }
    }
}

impl Settings {
    /// Inclusive lower bound for `catalog_stale_banner_days`.
    pub const CATALOG_STALE_DAYS_MIN: u32 = 1;
    /// Inclusive upper bound for `catalog_stale_banner_days`.
    pub const CATALOG_STALE_DAYS_MAX: u32 = 365;
    /// Inclusive lower bound for `trending_ttl_minutes`.
    pub const TRENDING_TTL_MIN: u32 = 5;
    /// Inclusive upper bound for `trending_ttl_minutes`.
    pub const TRENDING_TTL_MAX: u32 = 1440;
    /// Phase 15 — maximum entries kept in [`Self::skipped_update_versions`].
    /// Push beyond this evicts the oldest entry (FIFO) so the list
    /// can't grow without bound across decades of releases.
    pub const SKIPPED_UPDATE_VERSIONS_CAP: usize = 10;

    /// Apply the numeric clamps declared in the field docs. Idempotent;
    /// safe to call on already-clamped values.
    pub fn clamp(&mut self) {
        self.catalog_stale_banner_days = self
            .catalog_stale_banner_days
            .clamp(Self::CATALOG_STALE_DAYS_MIN, Self::CATALOG_STALE_DAYS_MAX);
        self.trending_ttl_minutes = self
            .trending_ttl_minutes
            .clamp(Self::TRENDING_TTL_MIN, Self::TRENDING_TTL_MAX);
        // Enforce the cap on every load/save in addition to the push
        // helper so a hand-edited settings.json with 50 skip entries
        // gets pruned on read.
        if self.skipped_update_versions.len() > Self::SKIPPED_UPDATE_VERSIONS_CAP {
            let excess = self.skipped_update_versions.len() - Self::SKIPPED_UPDATE_VERSIONS_CAP;
            self.skipped_update_versions.drain(..excess);
        }
    }

    /// Phase 15 — push `version` onto [`Self::skipped_update_versions`]
    /// with dedupe-and-move-to-tail semantics. Returns `true` when the
    /// list was actually mutated (entry was missing or at a different
    /// position); `false` when the entry was already at the tail.
    ///
    /// The cap is enforced by [`Self::clamp`] which is called by
    /// every `persist`. Callers don't need to clamp explicitly after
    /// pushing.
    pub fn push_skipped_version(&mut self, version: String) -> bool {
        // De-duplicate: drop any existing entry for this version so the
        // push always moves it to the tail.
        let already_at_tail = self
            .skipped_update_versions
            .last()
            .is_some_and(|v| v == &version);
        if already_at_tail {
            return false;
        }
        self.skipped_update_versions.retain(|v| v != &version);
        self.skipped_update_versions.push(version);
        while self.skipped_update_versions.len() > Self::SKIPPED_UPDATE_VERSIONS_CAP {
            self.skipped_update_versions.remove(0);
        }
        true
    }
}

/// Legacy icon-fetching mode inherited from the source app. Kept in
/// the settings schema for compatibility until the settings model is
/// pruned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaskIconMode {
    Off,
    InstalledOnly,
    All,
}

impl Default for CaskIconMode {
    fn default() -> Self {
        Self::All
    }
}

/// Three-state container that distinguishes file-absent (defaults apply)
/// from file-corrupt (fail closed — every outbound call denied until
/// the user repairs). `require_network` consults this on the first line
/// of every network-touching command.
#[derive(Debug, Clone)]
pub enum SettingsLoadState {
    /// No `settings.json` on disk yet (or stat failed with `NotFound`).
    /// All settings read as `Settings::default()`.
    FirstLaunch,
    /// Good parse — `Settings` is in memory, every field has been
    /// clamped to its declared bounds, paranoid_mode + other toggles
    /// reflect what the user picked.
    Loaded(Settings),
    /// File present but unreadable (stat failure, size cap exceeded,
    /// parse error, IO error). Fail closed: every gated command denies
    /// outbound until the user runs `settings_reset` (which materialises
    /// the file with defaults) or repairs the file by hand.
    Corrupt { message: String },
}

impl SettingsLoadState {
    /// Materialise a `Settings` for the cases that have one
    /// (`FirstLaunch` returns defaults, `Loaded(s)` returns `s`),
    /// returning `None` for `Corrupt` so callers can branch on
    /// "fall back to defaults" vs "block until repaired".
    pub fn effective_settings(&self) -> Option<Settings> {
        match self {
            Self::Loaded(s) => Some(s.clone()),
            Self::FirstLaunch => Some(Settings::default()),
            Self::Corrupt { .. } => None,
        }
    }
}
