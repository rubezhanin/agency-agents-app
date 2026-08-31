//! Tauri command surface for the audit log (Phase 5).
//!
//! Two IPCs:
//!   - `audit_log(entry)`   — append one entry. The frontend calls
//!                            this after a successful user-initiated
//!                            operation (install, hermes install,
//!                            runbook apply, settings update).
//!   - `audit_recent(limit)` — read the most-recent N entries,
//!                            newest first. Backs the Settings →
//!                            Activity / Audit log tab.

use serde::Deserialize;
use tauri::{AppHandle, Manager};

use crate::audit::{self, AuditEntry, AuditOutcome};
use crate::error::AppError;
use crate::state::AppState;

/// Append one entry to the audit log. The frontend fills `kind` and
/// `label` (and optional `targetId` + `detail`); the backend stamps
/// `timestamp` so the wall-clock comes from one place. The optional
/// `outcome` defaults to `ok`.
#[tauri::command]
pub async fn audit_log(
    request: AuditLogRequest,
    app: AppHandle,
) -> Result<(), AppError> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal {
            message: format!("audit_log: app_data_dir: {e}"),
        })?;
    let mut entry = request.entry;
    // Always stamp server-side so the clock comes from one place
    // and the log stays comparable across timezones.
    entry.timestamp = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        .to_rfc3339();
    audit::append(&app_data, &entry).await
}

/// Read up to `limit` entries, newest first. `limit` is clamped to
/// `[1, 500]` so a careless caller can't OOM the renderer.
#[tauri::command]
pub async fn audit_recent(
    limit: Option<usize>,
    app: AppHandle,
    _state: tauri::State<'_, AppState>,
) -> Result<Vec<AuditEntry>, AppError> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal {
            message: format!("audit_recent: app_data_dir: {e}"),
        })?;
    let limit = limit.unwrap_or(50).clamp(1, 500);
    audit::read_recent(&app_data, limit).await
}

/// Request body for `audit_log`. The frontend sends the entry
/// shape directly; the wrapper struct exists so adding request-only
/// fields later (e.g. `dry_run: bool`) doesn't break the wire.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogRequest {
    #[serde(default)]
    pub entry: AuditEntry,
}

// `AuditOutcome` is `pub` for the integration test; keep the import
// in scope so the symbol is referenced and rustfmt doesn't drop it.
#[allow(dead_code)]
fn _ensure_outcome_in_scope() -> AuditOutcome {
    AuditOutcome::Ok
}
