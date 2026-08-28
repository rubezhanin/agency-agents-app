//! GitHub IPC constants — scope requirements.
//!
//! v0.2.2 split this from a single constant after discovering that
//! GitHub's `PUT /repos/{o}/{r}/subscription` (the watch endpoint)
//! requires `notifications` specifically — `public_repo` alone returns
//! HTTP 404 (their privacy-preserving mask for "you don't have the
//! scope"). The action gate now checks the per-action required scope,
//! and the typed `ScopeRequired { scope }` error carries the SPECIFIC
//! scope name so the frontend can render an actionable "Re-authorize"
//! toast that triggers an incremental scope grant (signIn() with the
//! full GITHUB_OAUTH_SCOPES list — GitHub's consent screen surfaces
//! only the new scope, the existing ones display as "already granted").

/// `public_repo` — required for star/unstar/is_starred/create_issue.
/// Matches the OAuth scope the GitHub Device Flow requests during
/// sign-in.
pub const SCOPE_PUBLIC_REPO: &str = "public_repo";

/// `notifications` — required for watch/unwatch. NOT implied by
/// `public_repo`; has to be requested explicitly.
pub const SCOPE_NOTIFICATIONS: &str = "notifications";
