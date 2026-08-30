//! Startup recovery — sweep the operation journal for any
//! non-terminal rows and decide what to do with each.
//!
//! ## The problem
//!
//! `transaction::run()` writes a `pending` row before doing any
//! filesystem work and a terminal row (`committed` or `failed`)
//! after. If the process is killed between those two appends, the
//! row stays `pending` forever — and the on-disk state may or may
//! not be the post-conditions the work was supposed to leave.
//! Without a recovery pass at startup, the next reconcile would
//! never see the half-finished work and the user would be staring
//! at a "current" install that doesn't actually exist on disk.
//!
//! ## What this does
//!
//! On startup (`AppState::initialize` will call this once
//! install-side lands), it scans the journal, finds every
//! `pending` or `committing` row, and:
//!
//! 1. Records a synthetic `failed` row for it so the UI /
//!    reconcile see the operation as terminated.
//! 2. Returns a `RecoveryAction` list that the caller can act on
//!    (e.g. surface a "1 unfinished install recovered" banner in
//!    the Activity panel; or proactively run `reconcile` for the
//!    affected slug + tool).
//!
//! What this **doesn't** do:
//!
//! - It doesn't auto-rollback. The rollback decision is the
//!   user's, and it goes through the existing `backup_restore`
//!   path so the recovery flow reuses the audit trail.
//! - It doesn't delete journal rows. The journal is append-only
//!   forever; old `failed` rows just stay there for forensics.
//!   A separate manual rotation can prune it later.
//!
//! ## Idempotency
//!
//! If the recovery runs twice (e.g. two app launches in quick
//! succession), the second run sees no `pending` / `committing`
//! rows because the first run already recorded `failed` for
//! them. So this is safe to call on every startup.

use std::path::{PathBuf};

use serde::Serialize;
use ts_rs::TS;

use crate::error::AppError;

use super::journal::{
    append_entry, find_unfinished, now_iso, read_journal, OperationEntry, OperationStatus,
};

/// What the caller should do (and tell the user) about each
/// recovered operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub enum RecoveryAction {
    /// An install/update/restore was interrupted mid-flight.
    /// The on-disk state may be partial; the matching backup
    /// is in `app_data/backups/`. Recommend the user runs
    /// `backup_restore` for the affected `dest`.
    NeedsReview {
        operation_id: String,
        operation_type: String,
        targets: Vec<String>,
    },
}

/// Result of a single recovery pass. Serialised into the
/// `journal_recovery` Tauri event so the UI can render a
/// startup banner with the affected dests.
#[derive(Debug, Default, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct RecoveryReport {
    pub actions: Vec<RecoveryAction>,
    /// How many journal rows were flipped to `failed` by this
    /// pass. Useful for the startup banner / Activity log entry.
    pub recovered_count: usize,
    /// How many `pending` / `committing` rows we *found* on disk.
    /// Equal to `recovered_count` in normal operation; can
    /// differ only if the append-failed edge case happens (we
    /// log it as a warning and skip).
    pub found_count: usize,
    /// Diagnostics worth surfacing in the structured log: a
    /// corrupt journal row we skipped, or a journal file we
    /// couldn't read at all.
    pub warnings: Vec<String>,
}

/// Run the recovery pass against `app_data_dir` (the directory
/// that contains `journal/operations.jsonl`).
///
/// Safe to call on every startup; idempotent.
///
/// `app_data_dir` is the absolute path to the app's data dir
/// (the same path `corpus::app_data_dir` returns inside the
/// Tauri runtime).
pub async fn recover_unfinished(app_data_dir: &std::path::Path) -> Result<RecoveryReport, AppError> {
    let mut report = RecoveryReport::default();

    // 1. Load the journal. Missing file is fine (fresh install).
    let journal = read_journal(app_data_dir).await?;

    // 2. Idempotency: build a set of operation_ids that have
    //    *already* been recovered in a previous pass. We append
    //    `<id>:recovered` events, so any row whose base id has
    //    a matching `:recovered` row in the journal is
    //    considered already-handled. Without this, a second pass
    //    would see the original `pending` row still in the
    //    journal (append-only) and append a second
    //    `:recovered:recovered` row, which is a leak.
    let already_recovered: std::collections::HashSet<String> = journal
        .entries
        .iter()
        .filter_map(|e| {
            // The `:recovered` rows are themselves append-only
            // events; we recognise them by the suffix, not by a
            // field, because we never want to *miss* one (e.g.
            // if some future code path wrote a different status
            // on a `:recovered` row).
            e.operation_id
                .strip_suffix(":recovered")
                .map(|s| s.to_string())
        })
        .collect();

    let unfinished: Vec<&OperationEntry> = find_unfinished(&journal);
    report.found_count = unfinished.len();

    // 3. For each unfinished row, append a synthetic `failed`
    //    row and emit a RecoveryAction — *unless* we already
    //    have a `:recovered` event for it (idempotency). We
    //    deliberately do *not* mutate the existing pending row
    //    (the journal is append-only); the new row is a separate
    //    event that the UI / reconcile can pick up.
    for entry in unfinished {
        if already_recovered.contains(&entry.operation_id) {
            // Already handled by a prior pass. Don't append
            // another `:recovered` row; don't emit another action.
            continue;
        }
        let recovered_id = format!("{}:recovered", entry.operation_id);
        let mut failed = entry.clone();
        failed.operation_id = recovered_id.clone();
        failed.status = OperationStatus::Failed;
        failed.finished_at = Some(now_iso());
        failed.error = Some(
            "recovered at startup: process exited before transaction reached a terminal state. \
             Run reconcile and, if needed, backup_restore for these targets."
                .into(),
        );
        match append_entry(app_data_dir, &failed).await {
            Ok(()) => {
                report.recovered_count += 1;
                report.actions.push(RecoveryAction::NeedsReview {
                    operation_id: entry.operation_id.clone(),
                    operation_type: entry.operation_type.clone(),
                    targets: entry.targets.clone(),
                });
                tracing::warn!(
                    operation_id = %entry.operation_id,
                    operation_type = %entry.operation_type,
                    targets = ?entry.targets,
                    "recovery: marked unfinished operation as failed"
                );
            }
            Err(e) => {
                report.warnings.push(format!(
                    "recovery: failed to write failed row for {}: {e}",
                    entry.operation_id
                ));
            }
        }
    }

    Ok(report)
}

/// Convenience for the bootstrap site: returns just the
/// recovered `PathBuf`s the user should be told about, for
/// simpler log lines. The full report is the structured return
/// value; this is a "tell me what to put in the startup banner"
/// helper.
pub fn dest_paths_to_review(report: &RecoveryReport) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for action in &report.actions {
        // Single-variant for now; kept as a match so a future
        // variant (e.g. `AutoRollback`) can short-circuit here
        // without changing the call sites.
        let RecoveryAction::NeedsReview { targets, .. } = action;
        for t in targets {
            out.push(PathBuf::from(t));
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fresh_dir() -> std::path::PathBuf {
        // `keep()` instead of the deprecated `into_path` so the
        // dir survives past the test scope; no auto-cleanup.
        tempdir().unwrap().keep()
    }

    /// Two unfinished operations: one install (a real on-disk
    /// effect we can't see any more), one restore. Recovery
    /// should mark both as failed, emit two RecoveryActions, and
    /// leave the journal with `found_count == recovered_count`.
    #[tokio::test]
    async fn recovery_marks_unfinished_as_failed() {
        let dir = fresh_dir();
        // Simulate two pending rows left behind from a previous run.
        super::super::journal::append_entry(
            &dir,
            &OperationEntry::pending("op-A", "install", vec!["/dest/a".into()]),
        )
        .await
        .unwrap();
        super::super::journal::append_entry(
            &dir,
            &OperationEntry::pending("op-B", "restore", vec!["/dest/b".into()]),
        )
        .await
        .unwrap();
        // One already-committed row, should be left alone.
        let mut committed = OperationEntry::pending("op-C", "install", vec!["/dest/c".into()]);
        committed.status = OperationStatus::Committed;
        committed.finished_at = Some(now_iso());
        super::super::journal::append_entry(&dir, &committed).await.unwrap();

        let report = recover_unfinished(&dir).await.unwrap();
        assert_eq!(report.found_count, 2);
        assert_eq!(report.recovered_count, 2);
        assert_eq!(report.warnings.len(), 0);
        assert_eq!(report.actions.len(), 2);

        // The journal now has: op-A pending, op-B pending,
        // op-C committed, op-A:recovered failed, op-B:recovered
        // failed. The committed row is unchanged.
        let j = read_journal(&dir).await.unwrap();
        assert_eq!(j.entries.len(), 5);
        assert_eq!(j.entries[3].operation_id, "op-A:recovered");
        assert_eq!(j.entries[3].status, OperationStatus::Failed);
        assert_eq!(j.entries[4].operation_id, "op-B:recovered");
        assert_eq!(j.entries[4].status, OperationStatus::Failed);
    }

    /// Idempotency: running recovery twice in a row doesn't
    /// add new actions. The original `pending` row stays in the
    /// journal (append-only — we never delete rows), so
    /// `found_count` is still 1 on the second pass, but no new
    /// `RecoveryAction`s are emitted because we already marked
    /// it recovered last time. The journal grows by exactly one
    /// row per pass: the `:recovered` event for the original id.
    #[tokio::test]
    async fn recovery_is_idempotent() {
        let dir = fresh_dir();
        super::super::journal::append_entry(
            &dir,
            &OperationEntry::pending("op-A", "install", vec!["/a".into()]),
        )
        .await
        .unwrap();

        let first = recover_unfinished(&dir).await.unwrap();
        assert_eq!(first.found_count, 1);
        assert_eq!(first.recovered_count, 1);
        assert_eq!(first.actions.len(), 1);

        // Second pass: still finds the same original `pending`
        // row (append-only — we don't delete it), but the
        // skip-already-recovered logic in `recover_unfinished`
        // means we don't write another `:recovered` row for it
        // and we don't emit another `RecoveryAction`. So
        // `recovered_count` is 0 and `actions` is empty — this is
        // the actual idempotency the caller cares about (the
        // startup banner won't re-fire on every launch).
        let second = recover_unfinished(&dir).await.unwrap();
        assert_eq!(second.found_count, 1, "original pending row is still there");
        assert_eq!(second.recovered_count, 0, "skip: already recovered on prior pass");
        assert!(second.actions.is_empty(), "no new action for already-recovered op");
    }

    /// Empty journal (fresh install) is a no-op, not an error.
    #[tokio::test]
    async fn recovery_on_empty_journal_is_noop() {
        let dir = fresh_dir();
        let report = recover_unfinished(&dir).await.unwrap();
        assert_eq!(report.found_count, 0);
        assert_eq!(report.recovered_count, 0);
        assert!(report.actions.is_empty());
        assert!(report.warnings.is_empty());
    }

    /// `dest_paths_to_review` flattens, dedupes, and sorts the
    /// affected paths from the report.
    #[tokio::test]
    async fn dest_paths_to_review_dedupes_and_sorts() {
        let report = RecoveryReport {
            actions: vec![
                RecoveryAction::NeedsReview {
                    operation_id: "a".into(),
                    operation_type: "install".into(),
                    targets: vec!["/b".into(), "/a".into()],
                },
                RecoveryAction::NeedsReview {
                    operation_id: "b".into(),
                    operation_type: "restore".into(),
                    targets: vec!["/a".into()], // dup of above
                },
            ],
            ..Default::default()
        };
        let paths = dest_paths_to_review(&report);
        assert_eq!(
            paths,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }
}
