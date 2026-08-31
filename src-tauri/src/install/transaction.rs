//! `transaction()` — the wrapper every multi-step install / update /
//! uninstall / restore call sites should go through.
//!
//! ## Why
//!
//! Before this, a successful install was *implicit* — the call
//! site called `backup_if_differs`, then `atomic_write` per dest,
//! then `save_ledger`, then `record_backup_entries`, and a clean
//! exit was the only signal that everything had stuck. If the
//! process died in the middle, the on-disk state could be
//! inconsistent in ways the next startup couldn't see.
//!
//! With the operation journal, the only thing that needs to
//! change is: every call site that does the multi-step dance
//! now goes through `run()`. The journal records `pending` before
//! any work, `committed` after, or `failed` on error. Recovery
//! on startup (see `recovery.rs`) flips any `pending` /
//! `committing` rows that didn't reach a terminal state to
//! `failed` so the next reconcile / inspect surfaces the drift.
//!
//! ## The shape
//!
//! ```ignore
//! let op_id = uuid::Uuid::new_v4().to_string();
//! let targets = vec![dest_a, dest_b, dest_c];
//! let result = transaction::run(
//!     &app_data_dir,
//!     op_id,
//!     "install",
//!     targets.clone(),
//!     || async {
//!         backup_if_differs(&dest_a, &bytes_a, &backups, &stamp).await?;
//!         atomic_write(&dest_a, &bytes_a).await?;
//!         // ... and so on
//!         ledger.push(record);
//!         save_ledger(app, &ledger).await?;
//!         record_backup_entries(app, &pairs, &tool, &slug, &stamp).await?;
//!         Ok(record)
//!     },
//! )
//! .await?;
//! ```
//!
//! The closure returns `Result<T, AppError>` — `T` is whatever the
//! call site wants to return to the IPC handler. The wrapper
//! itself returns `Result<T, AppError>` unchanged; the only
//! difference is that the journal now knows whether the work
//! completed cleanly.

use std::path::Path;

use crate::error::AppError;

use super::journal::{
    append_entry, now_iso, OperationEntry, OperationStatus,
};

/// Run `work` and journal the result. The closure is the only
/// thing that touches the filesystem / ledger / backup index; the
/// wrapper itself only writes to the journal.
///
/// See the module doc for the contract.
pub async fn run<F, Fut, T>(
    app_data_dir: &Path,
    operation_id: &str,
    operation_type: &str,
    targets: Vec<String>,
    work: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, AppError>>,
{
    // 1. Pending: the work hasn't started yet. If the process
    //    dies between this and the next append, recovery marks us
    //    as `failed`.
    let started_at = now_iso();
    let pending = OperationEntry {
        operation_id: operation_id.to_string(),
        operation_type: operation_type.to_string(),
        status: OperationStatus::Pending,
        targets: targets.clone(),
        started_at: started_at.clone(),
        finished_at: None,
        error: None,
    };
    append_entry(app_data_dir, &pending).await?;

    // 2. Run the work. The closure is the call site's business
    //    logic; the wrapper only journals the outcome.
    match work().await {
        Ok(value) => {
            // 3a. Committed. The work's post-conditions already passed
            //     inside the closure (ledger was saved, backups
            //     were recorded, etc.) — we just declare victory.
            let committed = OperationEntry {
                operation_id: operation_id.to_string(),
                operation_type: operation_type.to_string(),
                status: OperationStatus::Committed,
                targets,
                started_at,
                finished_at: Some(now_iso()),
                error: None,
            };
            // If this append fails, we still return Ok to the
            // caller — the filesystem state is good, only the
            // journal write failed (e.g. disk full mid-write). The
            // recovery code is robust to "we have a missing
            // terminal row" because the absence of a terminal row
            // is itself a recoverable state.
            if let Err(e) = append_entry(app_data_dir, &committed).await {
                tracing::warn!(
                    operation_id,
                    error = %e,
                    "journal: failed to write committed row after successful work; \
                     caller will see Ok but recovery will need to inspect filesystem state"
                );
            }
            Ok(value)
        }
        Err(work_err) => {
            // 3b. Failed. The work's own error is the one we report
            //     to the caller; the journal row's `error` field
            //     carries a display copy for the recovery UI.
            let failed = OperationEntry {
                operation_id: operation_id.to_string(),
                operation_type: operation_type.to_string(),
                status: OperationStatus::Failed,
                targets,
                started_at,
                finished_at: Some(now_iso()),
                error: Some(format!("{work_err}")),
            };
            if let Err(e) = append_entry(app_data_dir, &failed).await {
                tracing::warn!(
                    operation_id,
                    error = %e,
                    "journal: failed to write failed row after work error; \
                     caller will see Err but the journal may be missing the terminal row"
                );
            }
            Err(work_err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fresh_dir() -> std::path::PathBuf {
        // `keep()` instead of the deprecated `into_path`.
        tempdir().unwrap().keep()
    }

    /// Successful work: journal gets a pending then a committed
    /// row, no failed rows, caller's value is returned.
    #[tokio::test]
    async fn run_appends_committed_on_success() {
        let dir = fresh_dir();
        let targets = vec!["/dest/a".into(), "/dest/b".into()];
        let result: i32 = run(&dir, "op-1", "install", targets.clone(), || async {
            Ok(42)
        })
        .await
        .unwrap();
        assert_eq!(result, 42);
        let j = super::super::journal::read_journal(&dir).await.unwrap();
        assert_eq!(j.entries.len(), 2);
        assert_eq!(j.entries[0].status, OperationStatus::Pending);
        assert_eq!(j.entries[1].status, OperationStatus::Committed);
        assert!(j.entries[1].finished_at.is_some());
        assert!(j.entries[1].error.is_none());
    }

    /// Failed work: journal gets a pending then a failed row,
    /// caller's error is propagated.
    #[tokio::test]
    async fn run_appends_failed_on_error() {
        let dir = fresh_dir();
        let targets = vec!["/dest/a".into()];
        let result: Result<i32, AppError> = run(&dir, "op-2", "install", targets, || async {
            Err(AppError::Io {
                message: "simulated".into(),
            })
        })
        .await;
        assert!(matches!(result, Err(AppError::Io { .. })));
        let j = super::super::journal::read_journal(&dir).await.unwrap();
        assert_eq!(j.entries.len(), 2);
        assert_eq!(j.entries[0].status, OperationStatus::Pending);
        assert_eq!(j.entries[1].status, OperationStatus::Failed);
        assert!(j.entries[1].error.as_deref().unwrap().contains("simulated"));
    }

    /// Work that panics or is cancelled *before* it can write a
    /// terminal row: simulated by simulating a process exit. We
    /// don't actually call `std::process::exit` (would kill the
    /// test) — instead we just leave the pending row behind by
    /// not calling any code that would append a terminal row, and
    /// assert that `find_unfinished` returns it. This is the
    /// state recovery sweeps at startup.
    #[tokio::test]
    async fn pending_row_left_behind_is_recoverable() {
        let dir = fresh_dir();
        let pending = OperationEntry::pending("op-3", "install", vec!["/x".into()]);
        append_entry(&dir, &pending).await.unwrap();
        // No terminal row written — the process "died" here.
        let j = super::super::journal::read_journal(&dir).await.unwrap();
        let unfinished = super::super::journal::find_unfinished(&j);
        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0].operation_id, "op-3");
        assert_eq!(unfinished[0].status, OperationStatus::Pending);
    }

    /// A successful and a failed operation interleaved: the
    /// journal records both lifecycles independently. This is the
    /// common case (one install, one uninstall of a different
    /// agent) — the journal must not conflate them.
    #[tokio::test]
    async fn multiple_operations_keep_independent_lifecycles() {
        let dir = fresh_dir();
        // Discard the result of the closure (we only care that
        // the journal ends up with the right rows). The unit
        // type bound comes from `Ok(())` in the closure body.
        // Clippy's `let_unit_value` lint: omit the `let _ =`
        // since the future is already awaited and its return is
        // intentionally discarded.
        run(&dir, "op-A", "install", vec!["/a".into()], || async { Ok(()) })
            .await
            .unwrap();
        run(&dir, "op-B", "uninstall", vec!["/b".into()], || async { Ok(()) })
            .await
            .unwrap();
        let r: Result<(), AppError> = Err(AppError::Io {
            message: "boom".into(),
        });
        let _ = run(&dir, "op-C", "restore", vec!["/c".into()], || async { r }).await;
        let j = super::super::journal::read_journal(&dir).await.unwrap();
        // 3 operations × 2 rows each (pending + terminal) = 6 rows
        assert_eq!(j.entries.len(), 6);
        let types: Vec<&str> = j
            .entries
            .iter()
            .step_by(2)
            .map(|e| e.operation_type.as_str())
            .collect();
        assert_eq!(types, vec!["install", "uninstall", "restore"]);
        let terminals: Vec<&OperationStatus> =
            j.entries.iter().skip(1).step_by(2).map(|e| &e.status).collect();
        assert!(matches!(terminals[0], OperationStatus::Committed));
        assert!(matches!(terminals[1], OperationStatus::Committed));
        assert!(matches!(terminals[2], OperationStatus::Failed));
    }
}
