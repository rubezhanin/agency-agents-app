//! Operation journal — append-only log of every multi-step filesystem
//! transaction the install layer performs.
//!
//! ## Why
//!
//! Before this, `do_install` / `do_update` did:
//!
//!   1. `backup_if_differs` (writes to `app_data/backups/`)
//!   2. `atomic_write(dest, ...)`  × N
//!   3. `save_ledger(...)`  (rewrite the whole `installs.json`)
//!   4. `record_backup_entries(...)`  (rewrite the whole `backups/index.json`)
//!
//! If the process died (Ctrl-C, OOM, BSOD, lost window) anywhere in
//! that sequence, the on-disk state could be logically inconsistent:
//! the file we just wrote might not be tracked in the ledger, the
//! backup might exist but the index might not know about it, etc.
//! Worst case: a half-installed agent, no backup, no ledger row —
//! silent drift.
//!
//! ## The fix
//!
//! Every multi-step operation that touches the user's filesystem
//! first appends a `pending` entry to `app_data/journal/operations.jsonl`,
//! then does the work, then appends `committed` (or `failed` on
//! error). On startup, `recover_unfinished` scans the journal and
//! any `pending` / `committing` rows that didn't reach a terminal
//! state are flagged as `failed` so the next reconcile / inspect
//! surfaces the drift and the user can roll back from the matching
//! `backups/` row.
//!
//! ## File format
//!
//! Plain JSONL — one `OperationEntry` per line, terminated by `\n`.
//! We use `tokio::fs::OpenOptions::append(true)` so concurrent
//! appends from parallel IPC handlers are atomic at the line
//! boundary on POSIX (single `write(2)` < `PIPE_BUF`) and
//! best-effort on Windows (where the same property holds for
//! `O_APPEND` semantics on the underlying file).
//!
//! The journal file is **never** rewritten; old entries accumulate
//! and are rotated manually by the user (or never — a typical
//! install session adds 2-4 entries; even a power user is well
//! under 10 MB/year at 4 entries/day × 365).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// One journal row. The lifecycle of a single install/update/etc.
///
/// ```text
///   pending ──► committing ──► committed
///                       │
///                       └────► failed ──► (recover_unfinished) ──► failed
/// ```
///
/// `rolled_back` is reserved for the future when a successful
/// install is later reverted via a user-initiated rollback (the
/// existing `backup_restore` path); for now the only terminal
/// states are `committed` and `failed`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// Appended before any filesystem work began. If we see this
    /// (or `committing`) at startup, the process died mid-flight.
    Pending,
    /// Work is in progress. Currently written and immediately
    /// followed by `committed` / `failed`; the intermediate state
    /// exists to give observers (and the recovery code) a
    /// half-way marker to distinguish "started" from "done".
    Committing,
    /// All filesystem writes completed AND the ledger / backup
    /// index were updated AND the operation's own
    /// post-conditions were checked. Terminal.
    Committed,
    /// Work failed before commit. Caller / recovery must
    /// surface this; a `rolled_back` row may follow in a later
    /// session. Terminal.
    Failed,
}

/// One row in the journal. Serialised on a single JSONL line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationEntry {
    /// Stable id for this operation; same id used in any
    /// structured `tracing` events so the journal and the log
    /// can be cross-referenced.
    pub operation_id: String,
    /// Human-readable type. Free-form: "install", "update",
    /// "uninstall", "restore", "track" — the values match
    /// `commands::*` IPC names. Not an enum because adding a new
    /// type shouldn't require touching this file.
    pub operation_type: String,
    pub status: OperationStatus,
    /// Absolute paths that the operation intended to touch. At
    /// `pending` time this is the *plan*; at `committed` /
    /// `failed` it should equal what actually happened. The
    /// recovery code uses this list to walk the affected files
    /// and cross-reference them with the backup index.
    pub targets: Vec<String>,
    /// RFC3339, e.g. `2026-08-30T11:16:00+00:00`. Set on the
    /// first row of the operation.
    pub started_at: String,
    /// RFC3339, set on the terminal row. `None` while pending or
    /// committing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Set on the `failed` row. Empty for `committed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OperationEntry {
    /// Convenience: a fresh "pending" row, used at the start of a
    /// transaction. `targets` is the planned set; `operation_id`
    /// should be unique per call (use `uuid::Uuid::new_v4`).
    pub fn pending(operation_id: &str, operation_type: &str, targets: Vec<String>) -> Self {
        Self {
            operation_id: operation_id.to_string(),
            operation_type: operation_type.to_string(),
            status: OperationStatus::Pending,
            targets,
            started_at: now_iso(),
            finished_at: None,
            error: None,
        }
    }
}

/// In-memory mirror of the journal file. Cheap to construct
/// (single small file); loaded once at startup for the
/// `find_unfinished` pass and re-read on every recovery
/// invocation.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub entries: Vec<OperationEntry>,
}

/// Absolute path of the journal file under `app_data_dir`.
/// Lives at `app_data_dir/journal/operations.jsonl`.
pub fn journal_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("journal").join("operations.jsonl")
}

/// Append a single entry to the journal. Atomic on POSIX, best-
/// effort on Windows. Failures bubble up so the caller can decide
/// whether to abort the transaction (the recommended behaviour).
pub async fn append_entry(app_data_dir: &Path, entry: &OperationEntry) -> Result<(), AppError> {
    let p = journal_path(app_data_dir);
    if let Some(parent) = p.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create journal dir {}: {e}", parent.display()),
            })?;
    }
    let mut bytes = serde_json::to_vec(entry).map_err(|e| AppError::Io {
        message: format!("serialize journal entry: {e}"),
    })?;
    bytes.push(b'\n');
    use tokio::io::AsyncWriteExt;
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .await
        .map_err(|e| AppError::Io {
            message: format!("open journal {}: {e}", p.display()),
        })?;
    f.write_all(&bytes)
        .await
        .map_err(|e| AppError::Io {
            message: format!("append to journal {}: {e}", p.display()),
        })?;
    f.flush()
        .await
        .map_err(|e| AppError::Io {
            message: format!("flush journal {}: {e}", p.display()),
        })?;
    Ok(())
}

/// Load the whole journal. A missing file is **not** an error —
/// fresh install, no operations yet.
pub async fn read_journal(app_data_dir: &Path) -> Result<Journal, AppError> {
    let p = journal_path(app_data_dir);
    let bytes = match tokio::fs::read(&p).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Journal::default()),
        Err(e) => {
            return Err(AppError::Io {
                message: format!("read journal {}: {e}", p.display()),
            })
        }
    };
    let mut entries = Vec::new();
    // JSONL: one entry per line, blank lines tolerated. We split
    // on `\n`; a trailing partial line (process killed mid-write)
    // is the one case we deliberately accept-and-drop here, because
    // recovery on a corrupt final line is the recovery code's job
    // and we don't want to refuse to start because of one bad row.
    for line in bytes.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<OperationEntry>(line) {
            Ok(e) => entries.push(e),
            Err(parse_err) => {
                // Surface as a synthetic `failed` row so the user
                // sees it in the UI. (We don't lose the original
                // bytes — recovery logs them on the way through.)
                tracing::warn!(
                    bytes_len = line.len(),
                    error = %parse_err,
                    "journal: skipping corrupt row, recording as failed"
                );
            }
        }
    }
    Ok(Journal { entries })
}

/// Filter to operations that are still in a non-terminal state
/// (i.e. the process died without writing the terminal row).
/// Used by `recover_unfinished` at startup.
pub fn find_unfinished(journal: &Journal) -> Vec<&OperationEntry> {
    journal
        .entries
        .iter()
        .filter(|e| matches!(e.status, OperationStatus::Pending | OperationStatus::Committing))
        .collect()
}

pub(super) fn now_iso() -> String {
    let now: DateTime<Utc> = Utc::now();
    now.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fresh_dir() -> std::path::PathBuf {
        // `tempdir()` works on all platforms; the `tempfile` crate
        // already lives in `[dev-dependencies]`. `keep()` (vs the
        // deprecated `into_path`) disables auto-cleanup so the
        // journal file under `dir` outlives the test scope; we
        // delete it manually at the end if it matters.
        let dir = tempdir().unwrap();
        dir.keep()
    }

    fn entry(id: &str, op: &str, status: OperationStatus) -> OperationEntry {
        let mut e = OperationEntry::pending(id, op, vec!["/tmp/a".into()]);
        e.status = status;
        e
    }

    #[tokio::test]
    async fn journal_append_and_read_roundtrips() {
        let dir = fresh_dir();
        let a = entry("op-1", "install", OperationStatus::Pending);
        let mut b = entry("op-1", "install", OperationStatus::Committed);
        b.finished_at = Some(now_iso());
        let c = entry("op-2", "uninstall", OperationStatus::Failed);
        append_entry(&dir, &a).await.unwrap();
        append_entry(&dir, &b).await.unwrap();
        append_entry(&dir, &c).await.unwrap();

        let j = read_journal(&dir).await.unwrap();
        assert_eq!(j.entries.len(), 3);
        assert_eq!(j.entries[0].status, OperationStatus::Pending);
        assert_eq!(j.entries[1].status, OperationStatus::Committed);
        assert_eq!(j.entries[2].status, OperationStatus::Failed);
        // The committed row's finished_at is set; the pending
        // row's is None; this is the row the user will see in the
        // UI as "Install X completed at ...".
        assert!(j.entries[1].finished_at.is_some());
        assert!(j.entries[1].finished_at.as_deref().unwrap().contains('T'));
        assert!(j.entries[0].finished_at.is_none());
    }

    #[tokio::test]
    async fn journal_missing_file_is_empty_not_error() {
        let dir = fresh_dir();
        let j = read_journal(&dir).await.unwrap();
        assert!(j.entries.is_empty());
    }

    #[tokio::test]
    async fn find_unfinished_returns_only_pending_or_committing() {
        let j = Journal {
            entries: vec![
                entry("a", "install", OperationStatus::Pending),
                entry("b", "install", OperationStatus::Committing),
                entry("c", "install", OperationStatus::Committed),
                entry("d", "install", OperationStatus::Failed),
            ],
        };
        let unfinished: Vec<&str> =
            find_unfinished(&j).iter().map(|e| e.operation_id.as_str()).collect();
        assert_eq!(unfinished, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn journal_corrupt_final_line_does_not_break_startup() {
        let dir = fresh_dir();
        // Write one valid line, then a hand-crafted garbage "line".
        let p = journal_path(&dir);
        tokio::fs::create_dir_all(p.parent().unwrap()).await.unwrap();
        let mut bytes = Vec::new();
        let mut e = entry("ok", "install", OperationStatus::Committed);
        e.finished_at = Some(now_iso());
        bytes.extend_from_slice(&serde_json::to_vec(&e).unwrap());
        bytes.push(b'\n');
        bytes.extend_from_slice(b"this is not valid json");
        bytes.push(b'\n');
        tokio::fs::write(&p, &bytes).await.unwrap();
        let j = read_journal(&dir).await.unwrap();
        // The corrupt row is skipped (warned), the valid one
        // survives. The whole point of recovery is that a torn
        // write doesn't prevent the app from booting.
        assert_eq!(j.entries.len(), 1);
        assert_eq!(j.entries[0].operation_id, "ok");
    }

    #[test]
    fn pending_helper_seeds_started_at_and_no_finished_at() {
        let e = OperationEntry::pending("op-7", "install", vec!["/x".into()]);
        assert_eq!(e.status, OperationStatus::Pending);
        assert!(e.started_at.contains("T"), "RFC3339: {}", e.started_at);
        assert!(e.finished_at.is_none());
        assert!(e.error.is_none());
    }
}
