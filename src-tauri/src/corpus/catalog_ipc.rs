//! Tauri IPC commands for the corpus subsystem.
//!
//! Split from `corpus/mod.rs` as stage C of the decomposition. The
//! data shape and the build/load/refresh path live in `corpus::catalog`;
//! the auto-detect / provision / pull helpers live in `corpus::catalog_detect`.
//! This file is the IPC surface that the frontend calls into.
//!
//! ## Conventions
//!
//! - All commands are `pub async fn` with `#[tauri::command]`.
//! - They take `&AppHandle` for filesystem access (`app_data_dir`,
//!   `catalog_root`) and `&State<AppState>` for the in-memory `Corpus`
//!   (read commands only — mutating commands call `state.rebuild_corpus()`
//!   which re-runs `resolve_active` + `persist`).
//! - Errors are returned as `AppError`; Tauri serialises that to the
//!   frontend as a typed error.
//!
//! ## Public surface
//!
//! - `corpus_status` / `corpus_refresh` — corpus build metadata + manual
//!   refresh from the GitHub tarball.
//! - `catalog_source_get` / `catalog_configured` / `catalog_source_set`—
//!   the persisted `CatalogSource` choice (Bundled / managed / user clone).
//! - `catalog_detect` / `catalog_provision_managed` / `catalog_pull` —
//!   the three actions of the catalog state-machine.
//! - `catalog_status` / `catalog_check_updates` — detailed catalog
//!   state (head/branch, ahead/behind vs upstream) for the status panel.
//! - `corpus_list` / `corpus_get` / `corpus_categories` — the read views
//!   served from the in-memory `Corpus` on `AppState`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::corpus::catalog_detect::{
    detect_catalogs, git_available, has_git_dir, provision_managed, pull_active, run_git,
};
use crate::corpus::catalog::{refresh, resolve_active};
use crate::corpus::source::{catalog_root, load_catalog_source, save_catalog_source};
use crate::error::AppError;
use crate::github::extract_github_repo;
use crate::util::sandbox::resolve_safe_path;
use crate::state::AppState;
use crate::types::{
    Agent, CatalogDetection, CatalogSource, CatalogStatus, CatalogUpdateCheck, Category,
    CorpusMeta,
};
use tauri::{AppHandle, State};

// `looks_like_catalog` is the small filesystem heuristic that decides
// whether a path a user is about to choose as their CatalogSource
// actually looks like an agency-agents catalog. It lives in
// `corpus::mod` (it's also consumed by `corpus::catalog_detect` and
// the corpus tests).
use crate::corpus::looks_like_catalog;

// =====================================================================
// Tauri commands (contracts.md §C — corpus surface)
// =====================================================================

// Note: the common helpers `baseline_dir` / `app_data_dir` / `ensure_corpus`
// that the IPC commands need live in `corpus/mod.rs` (they're shared with
// the install layer and the state wiring). They're imported below.

use crate::corpus::{app_data_dir, baseline_dir, ensure_corpus};
use crate::corpus::paths;

/// `corpus_status()` — version / commit / fetched-at / count for the
/// active corpus.
#[tauri::command]
pub async fn corpus_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    Ok(corpus.meta())
}

/// `corpus_refresh()` — fetch the live tarball, re-index, swap the
/// memoized corpus, and return the fresh meta.
#[tauri::command]
pub async fn corpus_refresh(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    state.require_network("corpus_refresh").await?;

    // Single-flight: a second click fast-fails rather than queuing a
    // duplicate download.
    let _flight = match state.corpus_refresh_in_flight.try_lock() {
        Ok(g) => g,
        Err(_) => {
            return Err(AppError::InvalidArgument {
                message: "corpus refresh already in progress".into(),
            });
        }
    };

    let adir = app_data_dir(&app)?;
    refresh(&adir).await?;

    // Rebuild the in-memory copy from the freshly-written working tree and
    // swap the memoized Arc so subsequent reads see the new corpus.
    let bdir = baseline_dir(&app)?;
    let fresh = Arc::new(resolve_active(&adir, &bdir).await);
    let meta = fresh.meta();
    {
        let mut cached = state.corpus_cache.lock().await;
        *cached = Some(fresh);
    }
    Ok(meta)
}

/// `catalog_source_get()` — the persisted [`CatalogSource`] (default Bundled).
#[tauri::command]
pub async fn catalog_source_get(app: AppHandle) -> Result<CatalogSource, AppError> {
    let adir = app_data_dir(&app)?;
    Ok(load_catalog_source(&adir).await)
}

/// `catalog_configured()` — whether the user has made an explicit catalog-source
/// choice yet (i.e. `state/catalog.json` exists). Drives the first-run prompt:
/// `false` ⇒ show the catalog-source picker before anything else.
#[tauri::command]
pub async fn catalog_configured(app: AppHandle) -> Result<bool, AppError> {
    let adir = app_data_dir(&app)?;
    Ok(self::paths::catalog_source_path(&adir).exists())
}

/// `catalog_source_set(source)` — switch where the catalog is read from, then
/// rebuild + swap the in-memory corpus so every view reflects the new source.
/// Validates that a `Managed`/`UserClone` path exists and looks like a catalog
/// (has at least one known category dir or `scripts/convert.sh`).
#[tauri::command]
pub async fn catalog_source_set(
    app: AppHandle,
    state: State<'_, AppState>,
    source: CatalogSource,
) -> Result<CorpusMeta, AppError> {
    // Validate non-bundled roots before committing to them.
    if let CatalogSource::Managed { path } | CatalogSource::UserClone { path, .. } = &source {
        let root = PathBuf::from(path);
        // Sandbox: canonicalise the path and require it to resolve
        // inside the user's home. This rejects `..` traversal
        // (e.g. `~/../etc/...`) and symlink escapes (a symlinked
        // directory whose target lies outside `home`).
        let home = dirs::home_dir().ok_or_else(|| AppError::Internal {
            message: "could not resolve home directory".into(),
        })?;
        let canonical = resolve_safe_path(&home, &root)?;
        if !canonical.is_dir() {
            return Err(AppError::InvalidArgument {
                message: format!("catalog path is not a directory: {path}"),
            });
        }
        if !looks_like_catalog(&canonical) {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "{path} doesn't look like an agency-agents catalog (no scripts/convert.sh or category dirs)"
                ),
            });
        }
    }

    let adir = app_data_dir(&app)?;
    save_catalog_source(&adir, &source).await?;
    rebuild_corpus(&app, &state).await
}

/// Rebuild the in-memory corpus from the currently-persisted source and swap
/// the memoized `Arc`, so every view reflects the latest catalog state. Shared
/// by source switching, provisioning, and pull.
pub(crate) async fn rebuild_corpus(app: &AppHandle, state: &AppState) -> Result<CorpusMeta, AppError> {
    let adir = app_data_dir(app)?;
    let bdir = baseline_dir(app)?;
    let fresh = Arc::new(resolve_active(&adir, &bdir).await);
    let meta = fresh.meta();
    {
        let mut cached = state.corpus_cache.lock().await;
        *cached = Some(fresh);
    }
    Ok(meta)
}

/// `catalog_detect(scan)` — discover candidate catalogs (always checks
/// `~/.agency-agents`; `scan=true` also walks common dev roots).
#[tauri::command]
pub async fn catalog_detect(scan: bool) -> Result<CatalogDetection, AppError> {
    Ok(detect_catalogs(scan).await)
}

/// `catalog_provision_managed()` — clone/snapshot into `~/.agency-agents`, set
/// it as the managed source, and rebuild. The "set one up for me" path.
#[tauri::command]
pub async fn catalog_provision_managed(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    state.require_network("catalog_provision_managed").await?;
    let path = provision_managed().await?;
    let adir = app_data_dir(&app)?;
    save_catalog_source(
        &adir,
        &CatalogSource::Managed {
            path: path.to_string_lossy().to_string(),
        },
    )
    .await?;
    rebuild_corpus(&app, &state).await
}

/// `catalog_pull()` — update the active catalog root (git pull or tarball
/// refresh), then rebuild. Rejected for a read-only user clone.
#[tauri::command]
pub async fn catalog_pull(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CorpusMeta, AppError> {
    state.require_network("catalog_pull").await?;
    let adir = app_data_dir(&app)?;
    pull_active(&adir).await?;
    rebuild_corpus(&app, &state).await
}

/// `catalog_status()` — provenance + freshness of the active catalog (source,
/// git commit/branch/dirty, remote repo, version, agent count). Local-only (no
/// network); the git fields are empty for a bundled/snapshot source.
#[tauri::command]
pub async fn catalog_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CatalogStatus, AppError> {
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source(&adir).await;
    let corpus = ensure_corpus(&app, &state).await?;
    let meta = corpus.meta();
    let root = catalog_root(&adir, &source);

    let is_git = has_git_dir(&root) && git_available().await;
    let mut branch = None;
    let mut commit = None;
    let mut last_commit_subject = None;
    let mut last_commit_date = None;
    let mut dirty_count = 0u32;
    let mut remote_url = None;
    let mut repo_slug = None;
    if is_git {
        let rs = root.to_string_lossy().to_string();
        branch = run_git(&["-C", &rs, "rev-parse", "--abbrev-ref", "HEAD"], None)
            .await
            .ok()
            .map(|s| s.trim().to_string());
        commit = run_git(&["-C", &rs, "rev-parse", "--short", "HEAD"], None)
            .await
            .ok()
            .map(|s| s.trim().to_string());
        if let Ok(log) = run_git(&["-C", &rs, "log", "-1", "--format=%s%x1f%cI"], None).await {
            let mut it = log.trim().splitn(2, '\u{1f}');
            last_commit_subject = it.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
            last_commit_date = it
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if let Ok(porcelain) = run_git(&["-C", &rs, "status", "--porcelain"], None).await {
            dirty_count = porcelain.lines().filter(|l| !l.trim().is_empty()).count() as u32;
        }
        remote_url = run_git(&["-C", &rs, "remote", "get-url", "origin"], None)
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        repo_slug = remote_url
            .as_deref()
            .and_then(extract_github_repo)
            .map(|r| format!("{}/{}", r.owner, r.repo));
    }

    let root_out = match source {
        CatalogSource::Bundled => None,
        _ => Some(root.to_string_lossy().to_string()),
    };

    Ok(CatalogStatus {
        source,
        root: root_out,
        is_git,
        branch,
        commit,
        last_commit_subject,
        last_commit_date,
        dirty_count,
        remote_url,
        repo_slug,
        version: meta.version,
        fetched_at: meta.fetched_at,
        agent_count: corpus.count(),
    })
}

/// `catalog_check_updates()` — fetch the active git catalog and report how far
/// behind/ahead upstream it is, plus a `git diff --stat` preview (the "stats on
/// diffs"). For a non-git source, returns `is_git=false` (the UI offers a plain
/// snapshot refresh instead). Network: runs `git fetch`.
#[tauri::command]
pub async fn catalog_check_updates(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CatalogUpdateCheck, AppError> {
    state.require_network("catalog_check_updates").await?;
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source(&adir).await;
    let root = catalog_root(&adir, &source);

    if !(has_git_dir(&root) && git_available().await) {
        return Ok(CatalogUpdateCheck {
            is_git: false,
            behind: 0,
            ahead: 0,
            changed_files: 0,
            diffstat: String::new(),
            up_to_date: false,
        });
    }

    let rs = root.to_string_lossy().to_string();
    run_git(&["-C", &rs, "fetch", "--quiet"], None).await?;

    // "<ahead>\t<behind>" relative to the upstream tracking branch.
    let (mut ahead, mut behind) = (0u32, 0u32);
    if let Ok(counts) = run_git(
        &[
            "-C",
            &rs,
            "rev-list",
            "--left-right",
            "--count",
            "HEAD...@{u}",
        ],
        None,
    )
    .await
    {
        let mut it = counts.split_whitespace();
        ahead = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        behind = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    }

    let (mut diffstat, mut changed_files) = (String::new(), 0u32);
    if behind > 0 {
        diffstat = run_git(&["-C", &rs, "diff", "--stat", "HEAD..@{u}"], None)
            .await
            .unwrap_or_default();
        if let Ok(names) = run_git(&["-C", &rs, "diff", "--name-only", "HEAD..@{u}"], None).await {
            changed_files = names.lines().filter(|l| !l.trim().is_empty()).count() as u32;
        }
    }

    Ok(CatalogUpdateCheck {
        is_git: true,
        behind,
        ahead,
        changed_files,
        diffstat,
        up_to_date: behind == 0,
    })
}

/// `corpus_list(category?)` — list view (bodies omitted).
#[tauri::command]
pub async fn corpus_list(
    app: AppHandle,
    state: State<'_, AppState>,
    category: Option<String>,
) -> Result<Vec<Agent>, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    Ok(corpus.list(category.as_deref()))
}

/// `corpus_get(slug)` — full agent incl. body.
#[tauri::command]
pub async fn corpus_get(
    app: AppHandle,
    state: State<'_, AppState>,
    slug: String,
) -> Result<Agent, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    corpus.get(&slug).ok_or(AppError::InvalidArgument {
        message: format!("unknown agent slug: {slug}"),
    })
}

/// `corpus_categories()` — the Discover grid (one tile per division declared
/// by the active catalog's tooling) with per-category counts.
#[tauri::command]
pub async fn corpus_categories(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<Category>, AppError> {
    let corpus = ensure_corpus(&app, &state).await?;
    Ok(corpus.categories())
}

