//! Catalog auto-detection, provisioning, and pull.
//!
//! Split from `corpus/mod.rs` so the catalog state-machine's three
//! actions are reviewable in isolation:
//!
//! - [`detect_catalogs`]: powers the "Find Agency Agents" button. Always
//!   inspects `~/.agency-agents`; with `scan: true` also walks the user's
//!   dev roots (`~/Software`, `~/Projects`, `~/git`, …) for an
//!   `agency-agents` checkout. Pure of app state — safe to call anytime.
//! - [`provision_managed`]: one-shot seed for the `~/.agency-agents`
//!   default. Clones via `git` when available, otherwise drops the
//!   GitHub tarball snapshot in place. Idempotent: a pre-existing
//!   catalog is left alone (use `pull_active` to update).
//! - [`pull_active`]: bring the active catalog root up to date.
//!   `git pull --ff-only` for checkouts, tarball swap for non-git
//!   sources. Read-only sources are rejected by the caller.
//!
//! ## Where the building blocks live
//!
//! The shared helpers used here (filesystem walk via `discover_categories`,
//! the catalog heuristic in `looks_like_catalog`, the empty-dir probe in
//! `is_empty_dir`, the tarball fetch in `download_corpus_tarball`, and the
//! full extract in `refresh`) currently live in `corpus::mod` and are
//! reached via `super::`. Stage B of the decomposition hoists
//! `download_corpus_tarball` + `refresh` into `corpus::catalog`, and at
//! that point this file's `super::refresh` becomes `super::super::catalog::refresh`
//! through the corpus re-export. (For now the `pub(super)` markers on the
//! mod-side are the contract.)

use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::types::{CatalogCandidate, CatalogDetection, CatalogSource};

use super::{
    bundled_division_slugs, discover_categories, download_corpus_tarball, is_empty_dir,
    looks_like_catalog, refresh,
};
use super::source::{catalog_root, load_catalog_source};
use super::tarball;

// ---------- Constants local to catalog detection ----------

/// Git remote used to clone/pull a managed catalog when `git` is available.
const CATALOG_GIT_URL: &str = "https://github.com/rubezhanin/agency-agents.git";

/// Dev-root directory names scanned (under `$HOME`) by the "Find Agency Agents"
/// button when looking for an existing clone.
const SCAN_ROOTS: [&str; 7] = [
    "Software",
    "Projects",
    "git",
    "Developer",
    "code",
    "dev",
    "src",
];


// ---------- Git / filesystem helpers ----------

pub(super) fn home_agency_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".agency-agents"))
}

/// Is a `git` binary on PATH? Determines clone/pull vs tarball-snapshot.
pub(super) async fn git_available() -> bool {
    run_git(&["--version"], None).await.is_ok()
}

/// Is `root` a git checkout (so a pull is `git pull`, not a tarball swap)?
pub(super) fn has_git_dir(root: &Path) -> bool {
    root.join(".git").exists()
}

/// Run `git` with `args` (optionally in `cwd`) off the async runtime. Errors
/// carry git's stderr so failures are diagnosable.
pub(super) async fn run_git(args: &[&str], cwd: Option<&Path>) -> Result<String, AppError> {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let cwd = cwd.map(|p| p.to_path_buf());
    let out = tokio::task::spawn_blocking(move || {
        let mut c = std::process::Command::new("git");
        if let Some(d) = &cwd {
            c.current_dir(d);
        }
        c.args(&owned).output()
    })
    .await
    .map_err(|e| AppError::Internal {
        message: format!("join git task: {e}"),
    })?
    .map_err(|e| AppError::Io {
        message: format!("spawn git: {e}"),
    })?;

    if !out.status.success() {
        return Err(AppError::Io {
            message: format!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Quick agent count for a candidate badge: top-level `.md` files across the
/// root's discovered categories. Cheap + synchronous (cold path, small repo).
pub(super) fn quick_agent_count(root: &Path) -> u32 {
    let mut n = 0u32;
    for cat in discover_categories(root) {
        if let Ok(rd) = std::fs::read_dir(root.join(&cat)) {
            n += rd
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
                .filter(|e| e.file_name().to_string_lossy() != "README.md")
                .count() as u32;
        }
    }
    n
}

/// Build a [`CatalogCandidate`] for `path` if it looks like a catalog.
pub(super) fn candidate_for(path: &Path, kind: &str) -> Option<CatalogCandidate> {
    if !looks_like_catalog(path) {
        return None;
    }
    Some(CatalogCandidate {
        path: path.to_string_lossy().to_string(),
        kind: kind.to_string(),
        has_git: has_git_dir(path),
        agent_count: quick_agent_count(path),
    })
}

/// Detect candidate catalogs. Always checks `~/.agency-agents`; when `scan` is
/// true also walks common dev roots for an `agency-agents` checkout (the "Find
/// Agency Agents" button). Pure of app state — safe to call anytime.
pub(super) async fn detect_catalogs(scan: bool) -> CatalogDetection {
    let git_available = git_available().await;
    let mut candidates: Vec<CatalogCandidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push = |c: Option<CatalogCandidate>,
                list: &mut Vec<CatalogCandidate>,
                seen: &mut std::collections::HashSet<String>| {
        if let Some(c) = c {
            if seen.insert(c.path.clone()) {
                list.push(c);
            }
        }
    };

    if let Some(managed) = home_agency_dir() {
        push(
            candidate_for(&managed, "managed"),
            &mut candidates,
            &mut seen,
        );
    }

    if scan {
        if let Some(home) = dirs::home_dir() {
            for root in SCAN_ROOTS {
                // Look for `<home>/<root>/agency-agents` and a direct
                // `<home>/<root>` that is itself a catalog.
                let base = home.join(root);
                push(
                    candidate_for(&base.join("agency-agents"), "userClone"),
                    &mut candidates,
                    &mut seen,
                );
                // One level of children named with "agency" (cheap heuristic).
                if let Ok(rd) = std::fs::read_dir(&base) {
                    for ent in rd.filter_map(|e| e.ok()) {
                        let p = ent.path();
                        if p.is_dir()
                            && p.file_name()
                                .map(|n| n.to_string_lossy().contains("agency"))
                                .unwrap_or(false)
                        {
                            push(candidate_for(&p, "userClone"), &mut candidates, &mut seen);
                        }
                    }
                }
            }
        }
    }

    CatalogDetection {
        git_available,
        scanned: scan,
        candidates,
    }
}

/// Ensure `~/.agency-agents` holds a catalog, cloning (git) or unpacking the
/// snapshot (no git) as needed. Returns the managed root path. Idempotent: if
/// it already looks like a catalog, this is a no-op (use pull to update).
pub(super) async fn provision_managed() -> Result<PathBuf, AppError> {
    let path = home_agency_dir().ok_or_else(|| AppError::Io {
        message: "cannot resolve home directory".into(),
    })?;
    if looks_like_catalog(&path) {
        return Ok(path); // already provisioned
    }

    let empty = is_empty_dir(&path);
    if git_available().await && !path.exists() {
        // git clone into a fresh dir (clone requires absent/empty target).
        // Full clone (not shallow) so commit history is available for accurate
        // behind/ahead counts and diff stats in the Catalog status panel.
        run_git(&["clone", CATALOG_GIT_URL, &path.to_string_lossy()], None).await?;
    } else if git_available().await && empty {
        // Full clone (not shallow) so commit history is available for accurate
        // behind/ahead counts and diff stats in the Catalog status panel.
        run_git(&["clone", CATALOG_GIT_URL, &path.to_string_lossy()], None).await?;
    } else {
        // No git (or a non-empty target): drop the snapshot tarball in place.
        tokio::fs::create_dir_all(&path)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create {}: {e}", path.display()),
            })?;
        let bytes = download_corpus_tarball().await?;
        let categories =
            self::tarball::categories_from_tarball(&bytes).unwrap_or_else(bundled_division_slugs);
        let written = self::tarball::extract_categories(&bytes, &path, &categories)?;
        if written == 0 {
            return Err(AppError::Internal {
                message: "provision: snapshot tarball contained no agent files".into(),
            });
        }
    }
    Ok(path)
}

/// Pull the active catalog root up to date. Git checkout → `git pull --ff-only`;
/// otherwise a tarball refresh into the root. Read-only sources are rejected by
/// the caller; Bundled refreshes its app-data copy.
pub(super) async fn pull_active(app_data_dir: &Path) -> Result<(), AppError> {
    let source = load_catalog_source(app_data_dir).await;
    if matches!(&source, CatalogSource::UserClone { manage: false, .. }) {
        return Err(AppError::InvalidArgument {
            message: "catalog source is read-only (manage-with-permission is off)".into(),
        });
    }
    let root = catalog_root(app_data_dir, &source);
    if has_git_dir(&root) && git_available().await {
        run_git(&["-C", &root.to_string_lossy(), "pull", "--ff-only"], None).await?;
        Ok(())
    } else {
        // Tarball refresh writes into the active root (refresh() resolves it).
        refresh(app_data_dir).await.map(|_| ())
    }
}
