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
use crate::hermes::{preflight_hermes, probe_hermes, HermesPreflight, HermesProbe, ProbeOptions};
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
    /// Optional plugin id (kebab-case). When `None` or empty, the
    /// canonical `agency-agents-router` plugin is installed (backward
    /// compatible). When set, the renderer writes the plugin to
    /// `~/.hermes/plugins/<plugin_id>/` with a custom manifest. Phase
    /// 4b — multi-plugin routing per division / per persona-set.
    #[serde(default)]
    pub plugin_id: Option<String>,
    /// Optional human-readable label, mirrored in `manifest.yaml`
    /// `display_name` and the router skill's `# Heading`. Required
    /// when `plugin_id` is set (default label is `Agency Agents Router`
    /// which is wrong for custom plugins). Ignored when `plugin_id`
    /// is `None` (the canonical label is hardcoded).
    #[serde(default)]
    pub plugin_label: Option<String>,
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

/// One row in the `hermes_list_plugins` response. The UI uses
/// `is_canonical` to mark the agency-agents-router plugin distinctly
/// (it cannot be deleted while the catalog is loaded — only refreshed
/// or uninstalled explicitly).
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct HermesInstalledPlugin {
    /// Plugin id (kebab-case). Equal to the directory basename and the
    /// `manifest.yaml` `id` field.
    pub plugin_id: String,
    /// Human-readable label from `manifest.yaml` `display_name`.
    pub label: String,
    /// Path to the on-disk plugin directory.
    pub path: PathBuf,
    /// Number of `skills/<slug>.md` files present (the persona count).
    pub agent_count: usize,
    /// True when this is the canonical `agency-agents-router` plugin.
    pub is_canonical: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HermesStageRequest {
    pub agents: Vec<RenderableAgent>,
    pub catalog_ref: String,
    pub dest: PathBuf,
    /// Optional plugin id. When set, the renderer uses the custom
    /// `manifest.yaml` `id:` field; otherwise the canonical
    /// `agency-agents-router` id is used. Ignored when staging to a
    /// user-picked directory (the directory name takes precedence).
    #[serde(default)]
    pub plugin_id: Option<String>,
    /// Optional human-readable label. See `HermesInstallRequest`.
    #[serde(default)]
    pub plugin_label: Option<String>,
}

/// Request shape for `hermes_uninstall`. Optional body — when omitted
/// (Tauri 2 sends `null`/`undefined`), the canonical
/// `agency-agents-router` plugin is removed. With a `pluginId` set,
/// the matching custom plugin is removed.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HermesUninstallRequest {
    #[serde(default)]
    pub plugin_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Probe the local `hermes` CLI: PATH lookup → scan-beyond-path → version.
#[tauri::command]
pub async fn hermes_status(_state: tauri::State<'_, AppState>) -> Result<HermesProbe, AppError> {
    Ok(probe_hermes(ProbeOptions::default()).await)
}

/// Run the Hermes pre-flight readiness check and return a structured
/// checklist (CLI, kanban, Node runtime, home writable, install target).
/// Informational only — the install buttons are not gated on the result.
#[tauri::command]
pub async fn hermes_preflight(
    _state: tauri::State<'_, AppState>,
) -> Result<HermesPreflight, AppError> {
    Ok(preflight_hermes().await)
}

/// List every installed Hermes plugin under
/// `~/.hermes/plugins/<plugin_id>/`. The scan is read-only and never
/// touches the plugin directories themselves; it just reads
/// `manifest.yaml` (or, when that file is missing, falls back to
/// `display_name = plugin_id` and `agent_count = count(skills/*.md)`).
///
/// Used by the Settings → Hermes tile to render the multi-plugin
/// table (Phase 4b) — the user can see which division plugins are
/// installed alongside the canonical `agency-agents-router` and
/// uninstall any of them.
#[tauri::command]
pub async fn hermes_list_plugins(
    _state: tauri::State<'_, AppState>,
) -> Result<Vec<HermesInstalledPlugin>, AppError> {
    let home = home_dir()?;
    let plugins_root = home.join(".hermes").join("plugins");
    if !plugins_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut out: Vec<HermesInstalledPlugin> = Vec::new();
    let entries = std::fs::read_dir(&plugins_root).map_err(|e| AppError::Io {
        message: format!(
            "hermes_list_plugins: read_dir {} failed: {e}",
            plugins_root.display()
        ),
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let plugin_id = match entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Skip dotfiles (staging dirs) and staging leftovers.
        if plugin_id.starts_with('.') {
            continue;
        }
        let manifest = path.join("manifest.yaml");
        let (label, declared_count) = if manifest.is_file() {
            read_manifest_summary(&manifest)
        } else {
            (plugin_id.clone(), None)
        };
        let skills = path.join("skills");
        let agent_count = if let Some(n) = declared_count {
            n
        } else if skills.is_dir() {
            std::fs::read_dir(&skills)
                .map(|d| {
                    d.flatten()
                        .filter(|e| {
                            e.path()
                                .extension()
                                .and_then(|x| x.to_str())
                                .map(|x| x.eq_ignore_ascii_case("md"))
                                .unwrap_or(false)
                        })
                        .count()
                })
                .unwrap_or(0)
        } else {
            0
        };
        out.push(HermesInstalledPlugin {
            is_canonical: plugin_id == hr::PLUGIN_ID,
            plugin_id,
            label,
            path,
            agent_count,
        });
    }
    out.sort_by(|a, b| {
        // Canonical first, then by id.
        b.is_canonical
            .cmp(&a.is_canonical)
            .then_with(|| a.plugin_id.cmp(&b.plugin_id))
    });
    Ok(out)
}

/// Best-effort `(display_name, declared_agent_count)` from
/// `manifest.yaml`. The renderer emits a hand-rolled YAML so we
/// parse it with a tiny line scanner rather than pull in serde_yaml
/// (determinism). Returns the plugin id and `None` for the count
/// when the manifest is missing fields.
fn read_manifest_summary(manifest_path: &std::path::Path) -> (String, Option<usize>) {
    let Ok(text) = std::fs::read_to_string(manifest_path) else {
        return (String::new(), None);
    };
    let mut display_name: Option<String> = None;
    let mut id: Option<String> = None;
    let mut skills_block = false;
    let mut skills_count: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.starts_with('#') {
            continue;
        }
        if !skills_block {
            if let Some(rest) = line.strip_prefix("display_name:") {
                display_name = Some(rest.trim().to_string());
                continue;
            }
            if let Some(rest) = line.strip_prefix("id:") {
                id = Some(rest.trim().to_string());
                continue;
            }
            if line.starts_with("skills:") {
                skills_block = true;
                continue;
            }
        } else if line.starts_with("- ") || line.starts_with("  - ") {
            // The renderer writes one `- id: <slug>` per skill under
            // a top-level `skills:` array. Count the bullet lines.
            *skills_count.get_or_insert(0) += 1;
        }
    }
    let label = display_name
        .or(id.clone())
        .unwrap_or_else(|| String::new());
    (label, skills_count)
}

/// Install a Hermes plugin. The plugin id defaults to
/// `agency-agents-router` (the canonical, full-catalog plugin) when
/// the request omits `pluginId`. For the multi-plugin routing path
/// (Phase 4b), the frontend passes a kebab-case `pluginId` + a
/// `pluginLabel` and the renderer writes the plugin to
/// `~/.hermes/plugins/<pluginId>/` with a custom manifest.
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

    let plugin = match (&request.plugin_id, &request.plugin_label) {
        (Some(id), Some(label)) if !id.is_empty() && !label.is_empty() => hr::render_named_plugin(
            &agents,
            &sources,
            &request.catalog_ref,
            &app_version,
            id,
            label,
        )?,
        (Some(id), _) if !id.is_empty() => {
            return Err(AppError::InvalidArgument {
                message: "hermes_install: plugin_label is required when plugin_id is set".into(),
            });
        }
        _ => hr::render_plugin(&agents, &sources, &request.catalog_ref, &app_version)?,
    };

    let dest = hr::user_install_root_for(&home_dir()?, &plugin.plugin_id);
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

/// Remove a previously-installed plugin directory. The plugin id is
/// optional — when omitted, removes the canonical
/// `agency-agents-router` plugin. Idempotent.
#[tauri::command]
pub async fn hermes_uninstall(
    request: Option<HermesUninstallRequest>,
    _state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let plugin_id = request
        .as_ref()
        .and_then(|r| r.plugin_id.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or(hr::PLUGIN_ID);
    let dest = hr::user_install_root_for(&home_dir()?, plugin_id);
    hr::uninstall_from(&dest)?;
    Ok(())
}

/// Stage the plugin into a user-picked directory (e.g. so they can
/// `hermes plugin install <path>`). The user picks the destination via a
/// file dialog from the frontend; the renderer writes the same plugin
/// bytes that `hermes_install` would. When `pluginId` is supplied the
/// staged manifest is labelled accordingly; otherwise the canonical
/// `agency-agents-router` id is used.
#[tauri::command]
pub async fn hermes_stage(request: HermesStageRequest) -> Result<HermesInstallResult, AppError> {
    let agents: Vec<Agent> = request.agents.into_iter().map(Agent::from).collect();
    let sources: Vec<String> = agents.iter().map(|a| a.body.clone()).collect();
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let plugin = match (&request.plugin_id, &request.plugin_label) {
        (Some(id), Some(label)) if !id.is_empty() && !label.is_empty() => hr::render_named_plugin(
            &agents,
            &sources,
            &request.catalog_ref,
            &app_version,
            id,
            label,
        )?,
        (Some(id), _) if !id.is_empty() => {
            return Err(AppError::InvalidArgument {
                message: "hermes_stage: plugin_label is required when plugin_id is set".into(),
            });
        }
        _ => hr::render_plugin(&agents, &sources, &request.catalog_ref, &app_version)?,
    };
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
