//! NEXUS scenario runbooks (`strategy/runbooks.json`).
//!
//! A runbook is a titled, mode-sized roster of catalog agents that
//! together ship a scenario — e.g. "Startup MVP Build" with a Core
//! Team, a Growth Team, etc. The app reads the manifest, resolves
//! each agent by slug against the local corpus, and can deploy the
//! whole set in one go.
//!
//! Extracted from `corpus/mod.rs` so the runbook schema + IPC live
//! in one place; the heavy `Corpus` struct + `resolve_active` stay
//! in `mod.rs`.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::error::AppError;

use super::app_data_dir;
use super::source::{catalog_root, load_catalog_source};

/// The `strategy/runbooks.json` manifest (catalog PR #664): machine-readable
/// NEXUS runbook rosters referenced BY SLUG (the corpus id / agent `.md` filename
/// stem), so the app resolves each to a catalog agent and can deploy the set.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct RunbooksFile {
    #[serde(default)]
    pub(crate) runbooks: Vec<Runbook>,
}

/// One NEXUS scenario runbook: a titled, mode-sized roster grouped into teams
/// (with activation timing), plus a pointer to its prose doc.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Runbook {
    pub slug: String,
    pub title: String,
    pub mode: String,
    pub duration: String,
    pub summary: String,
    pub doc: String,
    pub roster: Vec<RunbookGroup>,
}

/// A named sub-team within a runbook (e.g. "Core Team"), its activation timing,
/// and its member agents BY SLUG.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunbookGroup {
    pub group: String,
    pub activation: String,
    pub agents: Vec<String>,
}

/// `runbooks_list()` — the NEXUS runbook manifest from the active catalog's
/// `strategy/runbooks.json`. Empty when the catalog is the bundled snapshot or an
/// unsynced/pre-#664 clone (no `strategy/` on disk) — the UI treats empty as
/// "sync to unlock", not an error. Local-only (no network).
#[tauri::command]
pub async fn runbooks_list(app: AppHandle) -> Result<Vec<Runbook>, AppError> {
    let adir = app_data_dir(&app)?;
    let source = load_catalog_source(&adir).await;
    let root = catalog_root(&adir, &source);
    let path = root.join("strategy").join("runbooks.json");
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()), // no strategy/ (bundled / unsynced) → empty
    };
    let file: RunbooksFile = serde_json::from_str(&raw).map_err(|e| AppError::Io {
        message: format!("parse strategy/runbooks.json: {e}"),
    })?;
    Ok(file.runbooks)
}
