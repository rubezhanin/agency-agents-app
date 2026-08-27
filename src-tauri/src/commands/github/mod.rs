//! GitHub IPC surface (Phase 12c + 12e + 12f), split into submodules.
//!
//! - `types` — scope constants (`SCOPE_PUBLIC_REPO`, `SCOPE_NOTIFICATIONS`).
//! - `gates` — `authed_gate`, the 5-step pre-flight every Phase 12f
//!   authed action runs before any network call.
//! - `stats` — `github_repo_stats` (the hot path) + `github_status`.
//! - `auth` — `github_signin_start` / `github_signin_poll` /
//!   `github_signout`.
//! - `actions` — `github_star` / `github_unstar` / `github_is_starred`
//!   / `github_watch` / `github_unwatch` / `github_create_issue`.
//! - `tests` — integration tests for the paranoid / auth / scope /
//!   URL gate chain.
//!
//! Every command follows the same security pattern:
//! 1. Settings opt-in gate (for `github_repo_stats` only).
//! 2. Paranoid-mode gate (`require_network`).
//! 3. URL allowlist (`parse_github_url`).
//! 4. Auth gate (`auth::read_token`).
//! 5. Scope gate (per-action `required_scope`).
//!
//! See the module-level doc comment in each file for the specifics.

pub mod actions;
pub mod auth;
pub mod gates;
pub mod stats;
pub mod types;

#[cfg(test)]
mod tests;

// Public re-exports — the lib.rs `generate_handler!` list and the rest
// of the app import these by their previous, flat paths.
pub use actions::{
    github_create_issue, github_is_starred, github_star, github_unstar, github_unwatch,
    github_watch,
};
pub use auth::{github_signin_poll, github_signin_start, github_signout};
pub use stats::{github_repo_stats, github_status};
pub use types::{SCOPE_NOTIFICATIONS, SCOPE_PUBLIC_REPO};
