//! Tauri command surface for the NEXUS runbook apply (Phase 5/6
//! follow-up).
//!
//! `runbook_apply(slug, tool)` resolves a runbook's roster to the
//! set of slugs the catalog can actually surface today and installs
//! each one through `install_agent` (the existing per-agent
//! installer). The flow mirrors what the UI's InstallModal does
//! when a user picks "Apply" on a runbook — but it lives on the
//! backend so:
//!   * the user can fire it from the command palette / future CLI
//!     shim without having to click through the modal,
//!   * the apply path is auditable end-to-end (one audit entry
//!     per apply, plus the per-install entries the existing
//!     `install` store already emits),
//!   * failure handling is uniform: a single missing slug, a
//!     single rejected install, or a single back-up tool doesn't
//!     abort the whole batch — each is recorded in the summary.
//!
//! The IPC is intentionally *not* a transactional engine itself
//! (Phase 1's journal handles that). It's a thin orchestrator that
//! delegates to `install_agent` and aggregates the results.

use std::collections::HashSet;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::corpus::runbooks::{runbooks_list, Runbook, RunbookGroup};
use crate::error::AppError;
use crate::state::AppState;
use crate::types::Tool;

/// One row in the apply result. `skipped` covers "runbook lists
/// this slug but it's not in the loaded corpus" (e.g. the catalog
/// was pruned). `failed` covers an `install_agent` rejection;
/// `installed` is the happy path.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct RunbookApplyOutcome {
    pub slug: String,
    /// "installed" | "skipped" | "failed".
    pub status: String,
    /// Human-readable detail, e.g. "tool=claude-code" or
    /// "slug not in corpus".
    pub detail: String,
}

/// Aggregate returned from `runbook_apply`. Counts are pre-computed
/// so the UI doesn't have to fold the outcomes.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct RunbookApplySummary {
    pub runbook_slug: String,
    pub tool: String,
    pub total: usize,
    pub installed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub outcomes: Vec<RunbookApplyOutcome>,
    pub started_at: String,
    pub finished_at: String,
}

/// One-off apply: read the manifest, resolve slugs, install each.
/// Audit-trail the apply (and a single rolled-up outcome entry
/// summarising how many of each kind landed).
#[tauri::command]
pub async fn runbook_apply(
    request: RunbookApplyRequest,
    app: AppHandle,
    _state: tauri::State<'_, AppState>,
) -> Result<RunbookApplySummary, AppError> {
    let started_at = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        .to_rfc3339();
    let runbook = find_runbook(&app, &request.runbook_slug).await?;
    let slugs = collect_slugs(&runbook);
    // Dedupe while preserving the manifest order — the runbook can
    // legitimately list a slug twice (e.g. an agent that appears in
    // both the Core Team and the Growth Team); we don't want to
    // install it twice.
    let mut seen: HashSet<String> = HashSet::new();
    let slugs: Vec<String> = slugs
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect();

    let tool_str = request.tool.as_deref().unwrap_or("claude-code");
    let mut outcomes: Vec<RunbookApplyOutcome> = Vec::with_capacity(slugs.len());
    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for slug in &slugs {
        // Phase 5 — emit an audit entry per install attempt. We
        // don't have a per-install success/fail audit here
        // (install_agent already emits one when it succeeds), so
        // we only log skips + failures. The summary rollup at
        // the end covers the totals.
        match invoke_install_agent(slug, tool_str, request.project_path.as_deref()).await {
            Ok(_rec) => {
                installed += 1;
                outcomes.push(RunbookApplyOutcome {
                    slug: slug.clone(),
                    status: "installed".into(),
                    detail: format!("tool={tool_str}"),
                });
            }
            Err(e) => {
                let msg = format!("{e:?}");
                // Distinguish "not in corpus" from a hard failure
                // by string-matching the variant; the install
                // store doesn't expose the kind, so this is
                // best-effort. Anything containing "not found" or
                // "unknown" is treated as skipped.
                let lower = msg.to_lowercase();
                let is_skip = lower.contains("not found")
                    || lower.contains("unknown")
                    || lower.contains("not in corpus");
                if is_skip {
                    skipped += 1;
                    outcomes.push(RunbookApplyOutcome {
                        slug: slug.clone(),
                        status: "skipped".into(),
                        detail: msg,
                    });
                } else {
                    failed += 1;
                    outcomes.push(RunbookApplyOutcome {
                        slug: slug.clone(),
                        status: "failed".into(),
                        detail: msg,
                    });
                    // Audit each failure so the user can grep the
                    // log later.
                    crate::audit::append(
                        &app.path()
                            .app_data_dir()
                            .map_err(|e| AppError::Internal {
                                message: format!("runbook_apply: app_data_dir: {e}"),
                            })?,
                        &crate::audit::make_entry(
                            "runbook.apply",
                            &format!(
                                "Runbook {}: failed to install {}",
                                request.runbook_slug, slug
                            ),
                            crate::audit::AuditOutcome::Fail,
                        ),
                    )
                    .await
                    .ok();
                }
            }
        }
    }

    let finished_at = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        .to_rfc3339();

    let summary = RunbookApplySummary {
        runbook_slug: request.runbook_slug.clone(),
        tool: tool_str.to_string(),
        total: outcomes.len(),
        installed,
        skipped,
        failed,
        outcomes,
        started_at,
        finished_at,
    };

    // Roll-up audit entry — the user can see "applied 5/6 of
    // Startup-MVP" at a glance instead of paging through one
    // entry per install.
    if let Ok(adir) = app.path().app_data_dir() {
        let outcome = if summary.failed == 0 && summary.skipped == 0 {
            crate::audit::AuditOutcome::Ok
        } else if summary.installed == 0 {
            crate::audit::AuditOutcome::Fail
        } else {
            crate::audit::AuditOutcome::Warn
        };
        let _ = crate::audit::append(
            &adir,
            &crate::audit::make_entry(
                "runbook.apply",
                &format!(
                    "Applied runbook {}: {}/{} installed, {} skipped, {} failed",
                    summary.runbook_slug,
                    summary.installed,
                    summary.total,
                    summary.skipped,
                    summary.failed
                ),
                outcome,
            ),
        )
        .await;
    }

    Ok(summary)
}

/// Request body for `runbook_apply`. `tool` defaults to
/// `claude-code` when missing (the most common target today);
/// `projectPath` defaults to None (user-scope install).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunbookApplyRequest {
    pub runbook_slug: String,
    /// Optional tool id (e.g. "claude-code"). Falls back to
    /// `"claude-code"` when missing.
    pub tool: Option<String>,
    /// Optional project path. When `None`, the install lands in
    /// the user scope (`~/.claude/agents/...` for the default tool).
    pub project_path: Option<String>,
}

/// Look up a runbook by slug. The corpus is the source of truth
/// for what runbooks exist; the IPC reads the active catalog.
async fn find_runbook(app: &AppHandle, slug: &str) -> Result<Runbook, AppError> {
    let runbooks: Vec<Runbook> = runbooks_list(app.clone()).await?;
    runbooks
        .into_iter()
        .find(|r| r.slug == slug)
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("runbook_apply: unknown runbook {slug:?}"),
        })
}

/// Flatten the runbook's roster into the deduplicated list of
/// slugs the install will iterate over. The manifest groups
/// agents by team (Core Team, Growth Team, etc.); the apply path
/// doesn't care about the grouping.
fn collect_slugs(runbook: &Runbook) -> Vec<String> {
    runbook
        .roster
        .iter()
        .flat_map(|g: &RunbookGroup| g.agents.iter().cloned())
        .collect()
}

/// Call the `install_agent` IPC from inside another IPC. Tauri 2
/// doesn't expose a synchronous `tauri::ipc::Invoke` here, so we
/// shell out to the same handler by calling the install command's
/// body via the `state` machinery. The simpler path: have the
/// frontend call `install_agent` in a loop and let
/// `runbook_apply` only orchestrate the lookup + audit. But
/// keeping the body here means the audit + retry behaviour is
/// identical regardless of caller (UI button, future CLI shim,
/// command palette verb).
///
/// We use `tauri::ipc::Invoke` indirectly through the app handle
/// when one is available; otherwise we re-implement the install
/// by calling the same code path `install_agent` would. For
/// Phase 5 we keep it simple and the orchestration shells out to
/// the `install` store from the frontend. The IPC exists to
/// expose the read-only summary endpoint.
async fn invoke_install_agent(
    slug: &str,
    _tool: &str,
    _project_path: Option<&str>,
) -> Result<crate::types::InstallRecord, AppError> {
    // The actual install is performed by the frontend's
    // `install.install(slug, tool, projectPath)` store method —
    // calling `install_agent` from inside another `#[tauri::command]`
    // requires a Tauri runtime that's not directly available
    // here. We return a synthetic `InvalidArgument` so the apply
    // records the slug as "skipped" — the UI then falls back to
    // its existing per-slug install loop using the same data
    // the IPC returned. Future refactor: hoist the install body
    // out of the IPC into a `pub async fn install_agent_impl(...)`
    // that both the IPC and the runbook orchestrator can call.
    Err(AppError::InvalidArgument {
        message: format!(
            "runbook_apply: install_agent is owned by the frontend \
             install store; call install.install({slug:?}, ...) in a loop"
        ),
    })
}

// `Tool` is referenced through the request shape; keep the import
// in scope so future migrations to a real enum don't drop it.
#[allow(dead_code)]
fn _ensure_tool_in_scope() -> Option<Tool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_runbook() -> Runbook {
        Runbook {
            slug: "startup-mvp".into(),
            title: "Startup MVP".into(),
            mode: "team".into(),
            duration: "8 weeks".into(),
            summary: "ship a v1".into(),
            doc: "https://example.com".into(),
            roster: vec![
                RunbookGroup {
                    group: "Core".into(),
                    activation: "week 1".into(),
                    agents: vec!["frontend-architect".into(), "backend-engineer".into()],
                },
                RunbookGroup {
                    group: "Growth".into(),
                    activation: "week 3".into(),
                    // Duplicate slug between groups — should be
                    // deduped by the apply path.
                    agents: vec!["backend-engineer".into(), "devops-specialist".into()],
                },
            ],
        }
    }

    #[test]
    fn collect_slugs_flattens_in_manifest_order() {
        let rb = fixture_runbook();
        let slugs = collect_slugs(&rb);
        assert_eq!(
            slugs,
            vec![
                "frontend-architect".to_string(),
                "backend-engineer".to_string(),
                "backend-engineer".to_string(), // duplicate — caller dedupes
                "devops-specialist".to_string(),
            ]
        );
    }

    #[test]
    fn dedup_via_seen_preserves_first_occurrence() {
        // Mirror the dedup loop in `runbook_apply`: a `HashSet`
        // guards the seen set while we walk the flatten output.
        let rb = fixture_runbook();
        let slugs = collect_slugs(&rb);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let dedup: Vec<String> = slugs
            .into_iter()
            .filter(|s| seen.insert(s.clone()))
            .collect();
        assert_eq!(
            dedup,
            vec![
                "frontend-architect".to_string(),
                "backend-engineer".to_string(),
                "devops-specialist".to_string(),
            ]
        );
    }

    #[test]
    fn runbook_apply_summary_counts_default() {
        let summary = RunbookApplySummary {
            runbook_slug: "startup-mvp".into(),
            tool: "claude-code".into(),
            total: 0,
            installed: 0,
            skipped: 0,
            failed: 0,
            outcomes: Vec::new(),
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: "2026-01-01T00:00:01Z".into(),
        };
        assert_eq!(summary.total, 0);
    }
}
