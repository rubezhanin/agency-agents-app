/**
 * TypeScript equivalents of all Rust DTOs from `memory-bank/backendApi.md`.
 *
 * Camel-case JSON shape on the wire — these types match exactly what
 * `invoke()` returns for each Tauri command.
 */

// =========================================================
// 2.10 Settings (Phase 12d)
// =========================================================

/**
 * Legacy icon-fetching mode inherited from the source app. Kept in the
 * settings schema for compatibility until the settings model is pruned.
 */
export type CaskIconMode = "off" | "installed-only" | "all";

/**
 * Persisted user settings (Phase 12d). Lives at
 * `~/Library/Application Support/app.rubezhanin.agency-agents-app/settings.json` and is
 * round-tripped via `settingsGet` / `settingsSet`.
 *
 * Bounds (enforced server-side, also re-checked client-side for snappier
 * UX): `catalogStaleBannerDays` ∈ [1, 365]; `trendingTtlMinutes` ∈ [5, 1440].
 */
export interface Settings {
  /** Master switch — when true, every outbound command fails with
      `paranoid_mode_blocked`. */
  paranoidMode: boolean;
  catalogStaleBannerDays: number;
  caskIconMode: CaskIconMode;
  trendingTtlMinutes: number;
  /** Phase 12c — when true, PackageDetail probes `api.github.com` for
      repo stats whenever the package's homepage is a GitHub URL. Off
      by default; the user opts in via Settings → GitHub. Independent
      of sign-in (anonymous probes still get the 60/hr public limit). */
  githubEnabled: boolean;
  /** Phase 13 — master AI Features toggle. When false, ALL AI-derived
      data is hidden in the UI: categories (Phase 9), enrichment
      (Phase 13), donut chart, category pills, friendly names,
      summaries, use cases, similar packages, tags. Default true. */
  aiFeaturesEnabled: boolean;
  /** Phase 15 — when true, the backend's auto-check scheduler wakes
      every 24h and calls `update_check_now`. Default off; the user
      opts in via Settings → Network → Updates. Suppressed (no fetch)
      while Offline Mode is on, regardless of this flag. */
  updateAutoCheck: boolean;
  /** Legacy enhanced-trending toggle inherited from the source app.
      Retained for settings-file compatibility. */
  enhancedTrendingEnabled: boolean;
  /** Legacy vulnerability-scanning toggle inherited from the source app.
      Agency Agents does not currently run a vulnerability scanner. */
  vulnerabilityScanningEnabled: boolean;
  /** Legacy live-enrichment toggle inherited from the source app. Agency
      Agents currently reads metadata from the active AA catalog. */
  liveEnrichmentEnabled: boolean;
  /** Per-tool custom install base path (tool id → absolute base dir). When set,
      user-scope installs + detection resolve against it instead of the OS home
      (e.g. pointing Claude Code at a WSL home). Absent/empty = OS home. */
  toolPaths: Record<string, string>;
}

/** Defaults matching the Rust `Settings::default()`. Used when seeding
    the settings store before the first `settingsGet` resolves so the UI
    doesn't have to render an empty state. */
export const SETTINGS_DEFAULTS: Settings = {
  paranoidMode: false,
  catalogStaleBannerDays: 14,
  caskIconMode: "all",
  trendingTtlMinutes: 60,
  // Phase 12c — anonymous GitHub stats opt-in. Off by default per the
  // "zero outbound unless user consented" posture.
  githubEnabled: false,
  // Phase 13 — AI-enriched rendering. ON by default so users get the
  // friendly names, summaries, and categories out of the box.
  aiFeaturesEnabled: true,
  // Auto-check for new Agency Agents releases. Off by
  // default per the "zero outbound unless user consented" posture.
  updateAutoCheck: false,
  // Legacy retained field. Off by default.
  enhancedTrendingEnabled: false,
  // Legacy retained field. Off by default.
  vulnerabilityScanningEnabled: false,
  // Opt-in live refresh of categories + descriptions. Off by default; same
  // legacy live enrichment path.
  liveEnrichmentEnabled: false,
  // Per-tool custom install base paths. Empty by default (all tools use ~).
  toolPaths: {},
};

// =========================================================
// 2.11 GitHub (Phase 12c + 12e)
// =========================================================

/**
 * Anonymous (or token-authenticated) repo metadata fetched from
 * `api.github.com/repos/{owner}/{repo}`. The backend caches the
 * response on disk for 24h, keyed by the validated owner/repo pair.
 *
 * `null`-able fields are absent on real-world repos: a repo with no
 * GitHub release will have `lastReleaseTag === null`, a live repo
 * will have `archivedAt === null`, etc.
 */
export interface RepoStats {
  owner: string;
  repo: string;
  stars: number;
  forks: number;
  openIssues: number;
  lastReleaseTag: string | null;
  lastReleaseDate: string | null;
  archived: boolean;
  archivedAt: string | null;
  licenseSpdx: string | null;
  defaultBranch: string;
  primaryLanguage: string | null;
}

/**
 * Sign-in status surface returned by `githubStatus`.
 *
 * **Token is never on the wire** — only the derived "what can the
 * session do?" view is. See `github::auth::GithubStatusDto` in the
 * backend for the matching Rust struct and the regression test that
 * pins the wire shape.
 */
export interface GithubStatus {
  signedIn: boolean;
  username: string | null;
  scopes: string[];
}

/**
 * Result of `githubSigninStart` — payload the frontend uses to show
 * the user code and drive the polling loop.
 */
export interface DeviceFlowStart {
  /** Short human-readable code (e.g. `WDJB-MJHT`) to type at
      `verificationUri`. */
  userCode: string;
  /** URL to open in the browser (usually `github.com/login/device`). */
  verificationUri: string;
  /** Seconds until `deviceCode` expires. After this, polling will
      return `expired`. */
  expiresIn: number;
  /** Server-recommended polling cadence in seconds. Must be honoured. */
  interval: number;
  /** Opaque code passed to `githubSigninPoll`. Never shown to the user. */
  deviceCode: string;
}

/**
 * Discriminated union returned by each `githubSigninPoll` call.
 *
 * The `slowDown` variant means GitHub asked us to back off — the
 * frontend should double its polling interval before the next call,
 * per RFC 8628 §3.5.
 */
export type DeviceFlowPoll =
  | { kind: "pending" }
  | { kind: "slowDown" }
  | { kind: "approved"; username: string | null; scopes: string[] }
  | { kind: "denied" }
  | { kind: "expired" };

/**
 * Result of `githubCreateIssue` — the freshly-minted issue's number
 * and canonical `html_url`. Returned by the create-issue backend
 * command (Phase 12f). The frontend opens `htmlUrl` via `safeOpenUrl`
 * after a successful submission.
 */
export interface CreatedIssue {
  number: number;
  htmlUrl: string;
}

// =========================================================
// 2.12 Updater (Phase 15)
// =========================================================

/**
 * A newer Agency Agents version surfaced by the manifest at
 * `agency-agents-app.rubezhanin.app/updater.json`. Held by the updater store
 * once a check returns `available`. Matches the camelCase wire shape
 * the backend's `UpdateCheckOutcome::Available` flattens onto when
 * serde-tagged with `kind`.
 *
 * `notes` is the raw `notes` field from the manifest — free-form text
 * (we publish a "See release notes at <url>" sentence by default), so
 * UI renders it as-is.
 *
 * No `sha256` here: the manifest sha256 is verified inside the plugin
 * before signature verification; never exposed to the renderer.
 */
export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  pubDate: string | null;
  skipped: boolean;
}

/**
 * Tagged union returned by `update_check_now`. Matches the backend's
 * `UpdateCheckOutcome` serde shape exactly: `{ kind, ...fields }` with
 * the `Available` fields flattened next to the discriminator (not
 * nested under `.info`). The store narrows on `kind` and lifts the
 * flat fields into the strongly-typed `UpdateInfo` for downstream
 * consumers.
 *
 *   - `upToDate` — manifest version ≤ running version, nothing to show.
 *   - `available` — newer version exists (and isn't on the user's
 *     skip-list); UI surfaces the indicator + the install action.
 *
 * `blocked` is **not** a wire variant — Offline Mode surfaces as
 * `AppError::ParanoidModeBlocked` instead, so the toast routes
 * through the same channel as every other gated call.
 */
export type UpdateCheckOutcome =
  | { kind: "upToDate" }
  | {
      kind: "available";
      version: string;
      currentVersion: string;
      notes: string | null;
      pubDate: string | null;
      skipped: boolean;
    };

// =========================================================
// 3.3 Error model
// =========================================================

export type AppErrorPayload =
  | { code: "json_parse";         command: string; message: string; rawExcerpt: string }
  | { code: "io";                 message: string }
  | { code: "network";            url: string; message: string }
  | { code: "http_status";        url: string; status: number }
  | { code: "invalid_argument";   message: string }
  | { code: "internal";           message: string }
  | { code: "paranoid_mode_blocked"; feature: string }
  | { code: "github_rate_limited"; resetAt: number }
  | { code: "keychain_unavailable"; message: string }
  | { code: "auth_required" }
  | { code: "scope_required"; scope: string }
  | { code: "hash_mismatch"; expected: string; actual: string }
  | { code: "signature_verification_failed"; message: string }
  | { code: "downgrade_rejected"; current: string; target: string };

/** Type-narrowing helper: is the thrown value a AppErrorPayload? */
export function isAppError(e: unknown): e is AppErrorPayload {
  return (
    typeof e === "object" &&
    e !== null &&
    "code" in e &&
    typeof (e as { code: unknown }).code === "string"
  );
}

/** Human-readable message for an AppError. */
export function appErrorMessage(e: AppErrorPayload): string {
  switch (e.code) {
    case "json_parse":          return `Failed to parse output: ${e.message}`;
    case "io":                  return `I/O error: ${e.message}`;
    case "network":             return `Network error: ${e.message}`;
    case "http_status":         return `HTTP ${e.status} from ${e.url}`;
    case "invalid_argument":    return `Invalid argument: ${e.message}`;
    case "internal":            return `Internal error: ${e.message}`;
    case "paranoid_mode_blocked":
      return `Offline Mode is on — ${e.feature} is blocked. Disable it in Settings → Network.`;
    case "github_rate_limited": {
      const reset = e.resetAt > 0 ? new Date(e.resetAt * 1000).toLocaleTimeString() : "soon";
      return `GitHub API rate limit reached. Resets at ${reset}. Sign in to lift the limit.`;
    }
    case "keychain_unavailable":
      return `macOS Keychain unavailable: ${e.message}`;
    case "auth_required":
      return "Sign in to GitHub to use this feature.";
    case "scope_required":
      return `GitHub permission "${e.scope}" required. Sign in again to grant it.`;
    case "hash_mismatch":
      return `Update aborted: downloaded artifact hash didn't match the manifest (expected ${e.expected.slice(0, 12)}…, got ${e.actual.slice(0, 12)}…).`;
    case "signature_verification_failed":
      return `Update aborted: signature verification failed (${e.message}).`;
    case "downgrade_rejected":
      return `Update refused: ${e.target} is not newer than the installed version (${e.current}).`;
  }
}

// =========================================================
// Agency Agents — corpus subsystem (contracts.md §A)
// =========================================================
//
// Mirrors the Rust DTOs in `src-tauri/src/types.rs`. Wire shape is
// camelCase.

/**
 * An AI coding tool we can deploy an agent into. The id set is data-driven
 * (see the tool registry under `$lib/data/toolRegistry`, backed by
 * `src-tauri/data/tools/*.json`) and not enumerated here — adding a tool is
 * adding a JSON file, not touching a Rust enum or a TS union. The Rust
 * side is `pub type Tool = String;` in `src-tauri/src/types.rs`; ts-rs
 * doesn't model string aliases, so this stays hand-rolled (it would be
 * the only thing `export * from './types.generated'` couldn't replace).
 */
export type Tool = string;

// =========================================================
// Agency Agents — corpus subsystem (contracts.md §A)
// =========================================================
//
// The 18 DTOs that mirror the Rust types in `src-tauri/src/types.rs` are
// re-exported from the machine-generated `types.generated.ts` below.
// That file is produced by `cargo test --test ts_export` (which drives
// ts-rs's `export_all()` for every `#[derive(TS)]` type) and is committed
// to the repo. A CI drift check (`quality.yml` → `ts-rs-drift` job)
// re-runs the codegen and fails the build if the committed file
// diverges from what was just generated. The non-DTO surface below
// (Settings, GitHub Device Flow, AppError, Hermes plugin types, UI
// types) is hand-rolled because it doesn't live in the Rust `types.rs`.

export * from "./types.generated";

/** A named sub-team within a runbook (e.g. "Core Team"), its activation timing,
    and its member agents BY SLUG (the corpus id). */
export interface RunbookGroup {
  group: string;
  activation: string;
  agents: string[];
}

/** One NEXUS scenario runbook from the catalog's `strategy/runbooks.json`. */
export interface Runbook {
  slug: string;
  title: string;
  mode: string;
  duration: string;
  summary: string;
  doc: string;
  roster: RunbookGroup[];
}

// =========================================================
// UI-only types (frontend stores, command palette, etc.)
// =========================================================

export type SidebarSection =
  | "dashboard"
  | "personas"
  | "tools"
  | "teams"
  | "projects"
  | "runbooks"
  | "activity";

export type ThemePreference = "light" | "dark" | "system";

/** Settings modal subsection. Kept in sync with Settings.svelte's
    internal section list — use this when deep-linking via
    `ui.openSettings(section)`. */
export type SettingsSection =
  | "appearance"
  | "catalog"
  | "network"
  | "github"
  | "activity"
  | "about"
  | "hermes";

/** Command-palette item — a verb (action). */
export type PaletteItem =
  | { kind: "command"; id: string; label: string; shortcut?: string; section?: string; run: () => void | Promise<void> };

// =========================================================
// Hermes plugin types (0.4.0)
// =========================================================
//
// Mirrors the Rust DTOs in `src-tauri/src/commands/hermes.rs` and
// `src-tauri/src/hermes/probe.rs`. The plugin format itself is documented
// in `docs/HERMES-PLUGIN.md`.

/** Where the `hermes` CLI was located when it was last probed. */
export type HermesProbeSource = "path" | "scan" | "missing";

/**
 * Result of `hermes_status` — the local `hermes` CLI state. `found: false`
 * means the user has not installed the CLI yet (or it's not on PATH); the
 * app can still install the plugin directory, but the user will need to
 * run `hermes plugin install <path>` themselves.
 */
export interface HermesProbe {
  found: boolean;
  /** Absolute path to the `hermes` binary, or `null` if not found. */
  path: string | null;
  source: HermesProbeSource;
  /** Parsed `MAJOR.MINOR.PATCH` from `hermes --version`, or `null`. */
  version: string | null;
  meetsMinimum: boolean;
  /** The minimum `hermes` version we integrate with (currently `0.12.0`). */
  minimum: string;
  /** Path from `hermes config path`, or `null` if the CLI is missing. */
  configPath: string | null;
  /** Whether `hermes kanban` subcommand is available on this install. */
  kanbanAvailable: boolean;
  /** Profiles from `hermes profile list`. */
  profiles: string[];
  /** Tail of stderr from a failed probe, if any. */
  stderrTail: string | null;
}

/** A persona passed to `hermes_install` / `hermes_stage`. Body is the
    full source text from the catalog. Matches `RenderableAgent` in
    `src-tauri/src/commands/hermes.rs`. */
export interface RenderableAgent {
  slug: string;
  name: string;
  description: string;
  category: string;
  body: string;
}

/** Per-skill hash returned by the install. */
export interface HermesSkillHash {
  slug: string;
  hash: string;
}

/** Result of `hermes_install` / `hermes_stage`. */
export interface HermesInstallResult {
  manifestHash: string;
  routerHash: string;
  skillHashes: HermesSkillHash[];
  installRoot: string;
  agentCount: number;
}
