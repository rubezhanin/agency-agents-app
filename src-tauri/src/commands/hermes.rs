//! Tauri command surface for the Hermes plugin installer + CLI detection.
//!
//! These commands back the **Tools → Hermes** tile and the **Agents → Install
//! as Hermes plugin** button. They are intentionally thin: all the heavy
//! work lives in `crate::render::hermes` and `crate::hermes::{probe,scan}`.
//!
//! Commands:
//!   - `hermes_status`      — probe the `hermes` CLI (PATH + scan + version).
//!   - `hermes_install`     — install the router plugin to
//!                            `~/.hermes/plugins/agency-agents-router/`.
//!   - `hermes_uninstall`   — remove the installed plugin (idempotent).
//!   - `hermes_stage`       — stage the plugin to a user-picked directory
//!                            (for `hermes plugin install <path>`).
//!
//! All four are safe to call when `hermes` is not installed; `hermes_status`
//! returns a `HermesProbe` with `found: false`, and the install/stage
//! commands do NOT shell out to `hermes` — they just write the directory.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::hermes::{probe_hermes, HermesProbe, ProbeOptions};
use crate::render::hermes as hr;
use crate::state::AppState;
use crate::types::Agent;

/// Resolve `$HOME` (or `%USERPROFILE%` on Windows) for the current user.
/// The install destination `~/.hermes/plugins/agency-agents-router/` is
/// home-scoped, NOT app-data-scoped, so we resolve it from the user home
/// and not from `state.app_data_dir`.
fn home_dir() -> Result<PathBuf, AppError> {
    dirs::home_dir().ok_or_else(|| AppError::Internal {
        message: "could not resolve user home directory".into(),
    })
}

/// A persona as the frontend ships it to the backend. Mirrors
/// `src/lib/types.ts` `RenderableAgent` (the install + reconcile path
/// already passes this shape around; we accept the same one to keep the
/// Tauri IPC surface uniform).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderableAgent {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub body: String,
}

impl From<RenderableAgent> for Agent {
    fn from(r: RenderableAgent) -> Self {
        Agent {
            slug: r.slug,
            name: r.name,
            description: r.description,
            category: r.category,
            emoji: None,
            color: None,
            vibe: None,
            body: r.body,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HermesInstallRequest {
    pub agents: Vec<RenderableAgent>,
    /// The catalog git ref the agents were read from. Frozen in the manifest.
    pub catalog_ref: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HermesInstallResult {
    pub manifest_hash: String,
    pub router_hash: String,
    pub skill_hashes: Vec<HermesSkillHash>,
    pub install_root: PathBuf,
    pub agent_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HermesSkillHash {
    pub slug: String,
    pub hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HermesStageRequest {
    pub agents: Vec<RenderableAgent>,
    pub catalog_ref: String,
    pub dest: PathBuf,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Probe the local `hermes` CLI: PATH lookup → scan-beyond-path → version.
#[tauri::command]
pub async fn hermes_status(_state: tauri::State<'_, AppState>) -> Result<HermesProbe, AppError> {
    Ok(probe_hermes(ProbeOptions::default()).await)
}

/// Install the `agency-agents-router` plugin into the canonical user
/// location (`~/.hermes/plugins/agency-agents-router/`).
///
/// The renderer is **deterministic**: identical input produces
/// byte-identical bytes. The install is **atomic**: the directory is
/// staged to a temp path, fsync'd, then renamed over the destination.
/// On Windows (where rename-over-non-empty is rejected) the renderer
/// removes the existing dir first.
///
/// The caller is the frontend's `DeploymentMatrix` "Install as Hermes
/// plugin" button or the "Stage for `hermes plugin install`..." option
/// (which calls `hermes_stage` instead).
#[tauri::command]
pub async fn hermes_install(
    request: HermesInstallRequest,
    _state: tauri::State<'_, AppState>,
) -> Result<HermesInstallResult, AppError> {
    let agents: Vec<Agent> = request.agents.into_iter().map(Agent::from).collect();
    let sources: Vec<String> = agents.iter().map(|a| a.body.clone()).collect();
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let plugin = hr::render_plugin(&agents, &sources, &request.catalog_ref, &app_version)?;
    let dest = hr::user_install_root(&home_dir()?);
    let report = hr::install_to(&plugin, &dest)?;
    Ok(HermesInstallResult {
        manifest_hash: report.manifest_hash,
        router_hash: report.router_hash,
        skill_hashes: report
            .skill_hashes
            .into_iter()
            .map(|(slug, hash)| HermesSkillHash { slug, hash })
            .collect(),
        install_root: dest,
        agent_count: plugin.file_count(),
    })
}

/// Remove the installed plugin directory. Idempotent.
#[tauri::command]
pub async fn hermes_uninstall(_state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    let dest = hr::user_install_root(&home_dir()?);
    hr::uninstall_from(&dest)?;
    Ok(())
}

/// Stage the plugin into a user-picked directory (e.g. so they can
/// `hermes plugin install <path>`). The user picks the destination via a
/// file dialog from the frontend; the renderer writes the same plugin
/// bytes that `hermes_install` would.
#[tauri::command]
pub async fn hermes_stage(request: HermesStageRequest) -> Result<HermesInstallResult, AppError> {
    let agents: Vec<Agent> = request.agents.into_iter().map(Agent::from).collect();
    let sources: Vec<String> = agents.iter().map(|a| a.body.clone()).collect();
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let plugin = hr::render_plugin(&agents, &sources, &request.catalog_ref, &app_version)?;
    let report = hr::install_to(&plugin, &request.dest)?;
    Ok(HermesInstallResult {
        manifest_hash: report.manifest_hash,
        router_hash: report.router_hash,
        skill_hashes: report
            .skill_hashes
            .into_iter()
            .map(|(slug, hash)| HermesSkillHash { slug, hash })
            .collect(),
        install_root: request.dest,
        agent_count: plugin.file_count(),
    })
}
