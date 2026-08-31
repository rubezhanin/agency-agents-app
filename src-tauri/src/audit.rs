//! Append-only audit log for significant user-initiated operations.
//!
//! Persisted at `<app_data>/audit/operations.jsonl`, one JSON record
//! per line. Survives crashes (every write is `fsync`'d via the same
//! tokio OpenOptions::append pattern as the install journal). The
//! log is a flat list — there's no per-action compaction, no log
//! rotation; the UI reads the tail.
//!
//! Records carry enough context to reconstruct "what happened, when,
//! to what, with what result" without joining against the live
//! state. The schema is intentionally permissive: `kind` is a free
//! string (so adding a new audit-emitting IPC doesn't require a
//! schema bump), but the shape is fixed.
//!
//! Read access is via `read_recent(N)` — the last N records, newest
//! first. There's no streaming / range query yet; the file is
//! expected to stay small (a few hundred lines per user per month).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AppError;

/// One row in the audit log. Always serialised to a single line in
/// `operations.jsonl`; reads reverse the order so the UI gets newest
/// first without a full file sort.
#[derive(Debug, Default, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// ISO-8601 UTC timestamp (RFC 3339).
    pub timestamp: String,
    /// Stable, machine-readable action name. Convention: `<area>.<verb>`,
    /// e.g. `install.commit`, `hermes.install`, `runbook.apply`,
    /// `settings.update`. Free-form for new areas.
    pub kind: String,
    /// Free-form human label the UI can show in a list. Falls back to
    /// `kind` when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `ok` | `warn` | `fail` — outcome of the operation. `ok` covers
    /// both successful mutations and intentional no-ops.
    #[serde(default)]
    pub outcome: AuditOutcome,
    /// Stable id of the affected entity (slug, plugin_id, runbook
    /// slug, etc.). Optional — bulk operations may leave this empty
    /// and put the count in `detail` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Free-form context string the UI can render as a subtitle
    /// (e.g. "12 agents, 3 tools", "~/.hermes/plugins/engineering-team").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "lowercase")]
pub enum AuditOutcome {
    #[default]
    Ok,
    Warn,
    Fail,
}

/// Resolve `<app_data>/audit/operations.jsonl`, creating the `audit/`
/// dir if it doesn't exist. The file is allowed to not exist yet.
pub fn audit_path(app_data: &Path) -> PathBuf {
    app_data.join("audit").join("operations.jsonl")
}

/// Append a single entry. Atomic per line: `tokio::fs::OpenOptions`
/// with `append(true)`, `create(true)`, plus a trailing newline.
/// A previous bad write (truncated last line) is not silently
/// repaired — the read path handles the trailing garbage.
pub async fn append(app_data: &Path, entry: &AuditEntry) -> Result<(), AppError> {
    use tokio::io::AsyncWriteExt;
    let path = audit_path(app_data);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| AppError::Io {
            message: format!("audit: create_dir_all {}: {e}", parent.display()),
        })?;
    }
    let mut line = serde_json::to_string(entry).map_err(|e| AppError::Io {
        message: format!("audit: serialise entry: {e}"),
    })?;
    line.push('\n');
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| AppError::Io {
            message: format!("audit: open {}: {e}", path.display()),
        })?;
    f.write_all(line.as_bytes())
        .await
        .map_err(|e| AppError::Io {
            message: format!("audit: write {}: {e}", path.display()),
        })?;
    f.sync_all().await.map_err(|e| AppError::Io {
        message: format!("audit: sync {}: {e}", path.display()),
    })?;
    Ok(())
}

/// Convenience builder: timestamp = now, kind/label/target filled in.
pub fn make_entry(kind: &str, label: &str, outcome: AuditOutcome) -> AuditEntry {
    AuditEntry {
        timestamp: DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339(),
        kind: kind.into(),
        label: Some(label.into()),
        outcome,
        target_id: None,
        detail: None,
    }
}

/// Read up to `limit` most-recent entries, newest first. The file is
/// read in full (it's expected to stay small — a few hundred lines
/// per user per month) and reversed. Trailing lines that fail to
/// parse (e.g. a partial write from a hard kill) are skipped so the
/// reader never sees a panic.
pub async fn read_recent(app_data: &Path, limit: usize) -> Result<Vec<AuditEntry>, AppError> {
    let path = audit_path(app_data);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(AppError::Io {
                message: format!("audit: read {}: {e}", path.display()),
            })
        }
    };
    let mut entries: Vec<AuditEntry> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Best-effort: a corrupt line (partial write, manual edit
        // gone wrong) is dropped, not fatal. The log is durable
        // enough that we shouldn't lose a whole read over one
        // bad row.
        if let Ok(e) = serde_json::from_str::<AuditEntry>(trimmed) {
            entries.push(e);
        }
    }
    entries.reverse(); // newest first
    if entries.len() > limit {
        entries.truncate(limit);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn append_and_read_round_trip() {
        let dir = tempdir().unwrap();
        let e1 = make_entry("install.commit", "Install frontend-architect", AuditOutcome::Ok);
        let e2 = make_entry("hermes.install", "Hermes plugin installed", AuditOutcome::Ok);
        append(dir.path(), &e1).await.unwrap();
        append(dir.path(), &e2).await.unwrap();
        let got = read_recent(dir.path(), 10).await.unwrap();
        // Newest first.
        assert_eq!(got[0].kind, "hermes.install");
        assert_eq!(got[1].kind, "install.commit");
        assert_eq!(got.len(), 2);
    }

    #[tokio::test]
    async fn read_recent_respects_limit() {
        let dir = tempdir().unwrap();
        for i in 0..5 {
            let e = make_entry(&format!("x.test.{i}"), "x", AuditOutcome::Ok);
            append(dir.path(), &e).await.unwrap();
        }
        let got = read_recent(dir.path(), 3).await.unwrap();
        assert_eq!(got.len(), 3);
        // Newest 3.
        assert_eq!(got[0].kind, "x.test.4");
        assert_eq!(got[2].kind, "x.test.2");
    }

    #[tokio::test]
    async fn corrupt_trailing_line_is_dropped() {
        use tokio::io::AsyncWriteExt;
        let dir = tempdir().unwrap();
        let e = make_entry("ok", "ok", AuditOutcome::Ok);
        append(dir.path(), &e).await.unwrap();
        // Append a corrupt trailing line.
        let path = audit_path(dir.path());
        let mut f = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .unwrap();
        f.write_all(b"{not json\n").await.unwrap();
        let got = read_recent(dir.path(), 10).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "ok");
    }

    #[test]
    fn empty_dir_yields_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = tempdir().unwrap();
        let got = rt.block_on(read_recent(dir.path(), 10)).unwrap();
        assert!(got.is_empty());
    }
}
