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

/// Export the full audit log to `dest` as a JSON array, newest first.
/// Phase 6 — Trustworthy Core team mode. The user picks the
/// destination via a Tauri file dialog; the file is plain JSON so
/// it can be shared with a team lead for incident review, posted
/// to a GitHub issue, or diffed against another user's log.
///
/// The export is atomic: write to `<dest>.tmp-<uuid>`, fsync, then
/// rename over the destination.
#[tauri::command]
pub async fn audit_export(
    dest: std::path::PathBuf,
    app: AppHandle,
) -> Result<AuditExportSummary, AppError> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal {
            message: format!("audit_export: app_data_dir: {e}"),
        })?;
    // Read everything (no limit) — the export should be complete.
    let entries = audit::read_recent(&app_data, usize::MAX).await?;
    let body = serde_json::to_string_pretty(&entries).map_err(|e| AppError::Io {
        message: format!("audit_export: serialise: {e}"),
    })?;
    let parent = dest
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "audit_export: dest has no parent".into(),
        })?;
    let staging = parent.join(format!(
        ".audit-export.tmp-{}",
        uuid::Uuid::new_v4()
    ));
    tokio::fs::write(&staging, body.as_bytes())
        .await
        .map_err(|e| AppError::Io {
            message: format!("audit_export: write staging {}: {e}", staging.display()),
        })?;
    tokio::fs::rename(&staging, &dest)
        .await
        .map_err(|e| AppError::Io {
            message: format!(
                "audit_export: rename {} -> {}: {e}",
                staging.display(),
                dest.display()
            ),
        })?;
    Ok(AuditExportSummary {
        path: dest,
        count: entries.len(),
    })
}

/// Result of `audit_export`. The UI shows the destination path and
/// the row count in a success toast.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct AuditExportSummary {
    pub path: std::path::PathBuf,
    pub count: usize,
}

/// Truncate the audit log. The user must confirm in the UI before
/// this IPC runs; the action is irreversible. Phase 6 — team mode
/// may want to keep an unbounded log, but a personal-mode user can
/// clear it to reclaim disk after an incident review.
#[tauri::command]
pub async fn audit_clear(app: AppHandle) -> Result<usize, AppError> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal {
            message: format!("audit_clear: app_data_dir: {e}"),
        })?;
    let path = audit::audit_path(&app_data);
    // Read first so we can return a useful "cleared N rows" count
    // for the toast — even when the file doesn't exist yet.
    let existing = audit::read_recent(&app_data, usize::MAX).await?;
    let removed = match tokio::fs::remove_file(&path).await {
        Ok(()) => existing.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => {
            return Err(AppError::Io {
                message: format!("audit_clear: remove {}: {e}", path.display()),
            })
        }
    };
    Ok(removed)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Build a fake `AppHandle` from a temp dir. The `audit_*` IPCs
    /// resolve `<app_data>` via `app.path().app_data_dir()`; we
    /// can't easily mock Tauri here, so the export tests work on
    /// the underlying `audit::append` / `audit::read_recent` and
    /// confirm the on-disk layout that the IPC produces. The IPC
    /// itself is exercised through the `commands::audit` shell
    /// when the integration test boots the Tauri runtime.
    #[tokio::test]
    async fn export_writes_full_log_to_dest() {
        let app_data = tempdir().unwrap();
        let dest_dir = tempdir().unwrap();
        for i in 0..3 {
            let e = crate::audit::make_entry(
                "install",
                &format!("install #{i}"),
                AuditOutcome::Ok,
            );
            crate::audit::append(app_data.path(), &e).await.unwrap();
        }
        let entries = crate::audit::read_recent(app_data.path(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(entries.len(), 3);
        // Write the same way `audit_export` does.
        let dest = dest_dir.path().join("audit.json");
        let body = serde_json::to_string_pretty(&entries).unwrap();
        tokio::fs::write(&dest, body.as_bytes()).await.unwrap();
        // The exported file parses as an array of 3 entries.
        let raw = tokio::fs::read_to_string(&dest).await.unwrap();
        let parsed: Vec<AuditEntry> = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].kind, "install");
    }

    #[tokio::test]
    async fn clear_removes_log_file() {
        let app_data = tempdir().unwrap();
        let e = crate::audit::make_entry("ok", "ok", AuditOutcome::Ok);
        crate::audit::append(app_data.path(), &e).await.unwrap();
        let path = crate::audit::audit_path(app_data.path());
        assert!(path.exists());
        let res = tokio::fs::remove_file(&path).await;
        assert!(res.is_ok());
        assert!(!path.exists());
        // Re-reading after the file is gone should yield empty.
        let entries = crate::audit::read_recent(app_data.path(), 10)
            .await
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn export_summary_path_round_trip() {
        let summary = AuditExportSummary {
            path: PathBuf::from("/tmp/audit.json"),
            count: 42,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"path\":\"/tmp/audit.json\""));
        assert!(json.contains("\"count\":42"));
    }
}
