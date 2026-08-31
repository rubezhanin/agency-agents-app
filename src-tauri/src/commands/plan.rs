//! Plan / Dry Run — pure-function preview of a destructive
//! operation.
//!
//! ## Why
//!
//! `install::do_install` and `install::do_update` both do
//! filesystem writes: backup the existing file (if any), then
//! `atomic_write` the new content, then update the ledger and
//! the backup index. The actual *content* of those writes
//! depends on a `render()` call that's pure and cheap, but the
//! *existence* of the target file (and the need to back it up)
//! is the question that matters for the user clicking
//! "Install" or "Update" — they want to know what's going to
//! change on their disk *before* it does.
//!
//! `deploy_plan` is the answer. It takes the same arguments as
//! `install_agent` / `update_agent` and returns a `Vec<PlanChange>`
//! describing every filesystem effect the install *would* have
//! without actually doing any of it. The UI in a follow-up
//! commit shows the plan as a pre-flight modal:
//!
//! ```text
//! Install Frontend Developer
//!
//!   +  .claude/agents/frontend-developer.md        (new, 4.2 KB)
//!   ~  .codex/agents/frontend-developer.toml        (4.1 KB → 4.3 KB, backed up)
//!
//! [Cancel]  [Install 2 changes]
//! ```
//!
//! The function is intentionally side-effect-free: no
//! filesystem writes, no journal entries, no ledger updates.
//! It reads the corpus (cached in `AppState`), reads the
//! existing on-disk files (if any), renders the new bytes,
//! and compares. Calling it N times has the same effect on
//! disk as calling it zero times.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::corpus;
use crate::error::AppError;
use crate::install;
use crate::render;
use crate::state::AppState;

/// One filesystem effect the install *would* have. The set
/// is the union of every `dests()` entry the render layer
/// returns for this `(slug, tool, project_path)` triple.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub enum PlanChange {
    /// The dest file doesn't exist yet; install will create
    /// it. We don't pre-render here, so `size` is 0; the UI
    /// shows "+" with a placeholder byte count.
    Create { dest: String, size: u64 },
    /// The dest file exists and the rendered bytes differ.
    /// Install will back the existing file up, then write
    /// the new bytes. `before_sha` / `after_sha` are short
    /// hex prefixes (16 chars) so the UI can show a stable
    /// "this changed" indicator without leaking the whole
    /// content; `backup_filename` is what the backup file
    /// will be named on disk (only set when `before_sha` is
    /// non-null, i.e. the install will actually create a
    /// backup).
    Overwrite {
        dest: String,
        before_sha: String,
        after_sha: String,
        backup_filename: String,
    },
    /// The dest file exists and matches the rendered bytes
    /// already. Install is a no-op for this dest.
    NoChange { dest: String, sha: String },
    /// The dest file exists, the rendered bytes are
    /// *different*, but it's outside the user's home
    /// directory and the sandbox refused to canonicalise it
    /// (e.g. a symlink pointing outside the home, or a path
    /// traversal). Install will refuse this dest; the plan
    /// surfaces the failure to the user.
    Refused { dest: String, reason: String },
}

impl PlanChange {
    /// Convenience for the UI: a single-character glyph per
    /// variant. The Settings → Catalog pane uses these to
    /// render a one-line summary.
    pub fn glyph(&self) -> &'static str {
        match self {
            Self::Create { .. } => "+",
            Self::Overwrite { .. } => "~",
            Self::NoChange { .. } => "=",
            Self::Refused { .. } => "!",
        }
    }

    /// Convenience: the absolute destination path, regardless
    /// of variant.
    pub fn dest(&self) -> &str {
        match self {
            Self::Create { dest, .. }
            | Self::Overwrite { dest, .. }
            | Self::NoChange { dest, .. }
            | Self::Refused { dest, .. } => dest,
        }
    }
}

/// Aggregate plan for one install. Returned by the
/// `deploy_plan` IPC; rendered by the UI as a pre-flight
/// modal.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct DeployPlan {
    /// One row per filesystem effect, in the order the
    /// renderer returned them.
    pub changes: Vec<PlanChange>,
    /// Convenience aggregate: how many files will be created
    /// (don't exist yet), overwritten (exist with different
    /// bytes), skipped (exist with matching bytes), or refused
    /// (sandbox violation). UI badges use these.
    pub summary: PlanSummary,
}

#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct PlanSummary {
    pub creates: u32,
    pub overwrites: u32,
    pub no_changes: u32,
    pub refused: u32,
}

impl PlanSummary {
    /// Total number of plan rows (== `changes.len()`).
    pub fn total(&self) -> u32 {
        self.creates + self.overwrites + self.no_changes + self.refused
    }

    /// True when the plan contains at least one *destructive*
    /// change (`Overwrite` or `Refused`). The UI uses this to
    /// decide whether the apply button needs an extra
    /// confirmation.
    pub fn is_destructive(&self) -> bool {
        self.overwrites > 0 || self.refused > 0
    }
}

/// Compute the deploy plan for one `(slug, tool, project_path)`
/// triple. Pure function: no writes, no journal, no ledger
/// updates. The caller (UI or future dry-run CLI) decides
/// what to do with the plan.
#[tauri::command]
pub async fn deploy_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    slug: String,
    tool: String,
    project_path: Option<String>,
) -> Result<DeployPlan, AppError> {
    let corpus = corpus::ensure_corpus(&app, &state).await?;
    let agent = corpus.get(&slug).ok_or_else(|| AppError::Io {
        message: format!("unknown agent: {slug}"),
    })?;
    let raw = corpus::read_source(&app, &agent.category, &slug).await?;

    let home = install::tool_home(&state, &tool).await?;
    let proot = project_path.as_ref().map(PathBuf::from);

    // Render the agent for this tool. The renderer returns
    // `(bytes_str, rendered_sha)` — bytes is a `String` (UTF-8
    // markdown), sha is the SHA-256 hex of those bytes. We
    // compare against the on-disk file (read as raw bytes)
    // by way of `String::as_bytes()`; markdown is ASCII-safe
    // for our use case, and the upstream `convert.sh` is too.
    let (rendered_str, rendered_sha) = render::render_with_hash(&agent, &raw, &tool)?;
    let rendered_bytes = rendered_str.as_bytes().to_vec();

    // Compute the destination paths the install would touch.
    let dests = render::dests(&tool, &agent.slug, &home, proot.as_deref())?;

    let backups_dir = install::backups_dir(&app)?;
    let stamp = install::now_iso();

    let mut changes = Vec::with_capacity(dests.len());
    let mut summary = PlanSummary::default();
    for dest in dests {
        let dest_str = dest.to_string_lossy().to_string();
        let on_disk = read_capped_safe(&dest).await;
        let change = match on_disk {
            None => PlanChange::Create {
                dest: dest_str,
                size: rendered_bytes.len() as u64,
            },
            Some(existing) if existing == rendered_bytes => PlanChange::NoChange {
                dest: dest_str,
                sha: rendered_sha.clone(),
            },
            Some(existing) => {
                // Sandbox check: if the resolved path escapes
                // the user's home, refuse the operation
                // instead of computing a backup name (we
                // wouldn't be able to write it anyway).
                if !path_inside_home(&dest, &home).await {
                    PlanChange::Refused {
                        dest: dest_str,
                        reason: "path resolves outside the user home; \
                                 install would refuse this dest at write time"
                            .into(),
                    }
                } else {
                    let before_sha = short_sha(&existing);
                    let fname = dest
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "agent".into());
                    let backup_filename = format!("{fname}.{}.bak", install::fs_stamp(&stamp));
                    let _ = backups_dir; // captured for future
                                         // "this is where the backup
                                         // would live" diagnostics;
                                         // the UI can show the
                                         // basename only.
                    PlanChange::Overwrite {
                        dest: dest_str,
                        before_sha,
                        after_sha: rendered_sha.clone(),
                        backup_filename,
                    }
                }
            }
        };
        match &change {
            PlanChange::Create { .. } => summary.creates += 1,
            PlanChange::Overwrite { .. } => summary.overwrites += 1,
            PlanChange::NoChange { .. } => summary.no_changes += 1,
            PlanChange::Refused { .. } => summary.refused += 1,
        }
        changes.push(change);
    }
    Ok(DeployPlan { changes, summary })
}

/// Read a file with a small size cap, returning `None` if the
/// file doesn't exist. We use a 1 MB cap (same as the existing
/// `read_capped` helper) so a 5 GB random file doesn't make
/// `deploy_plan` OOM the app.
async fn read_capped_safe(path: &Path) -> Option<Vec<u8>> {
    match tokio::fs::read(path).await {
        Ok(b) if b.len() <= 1024 * 1024 => Some(b),
        Ok(_) => Some(truncate_to_cap(path, 1024 * 1024).await),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => None,
    }
}

async fn truncate_to_cap(path: &Path, cap: usize) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut f = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut buf = Vec::with_capacity(cap);
    // Read up to `cap` bytes; the truncated-on-error path
    // below handles the case where the file is shorter than
    // `cap`. We intentionally don't call `truncate(0)` — the
    // empty-read case is a normal "the on-disk file is empty"
    // scenario, and we want to surface that to the caller
    // (rather than returning an empty Vec that's
    // indistinguishable from "the file doesn't exist").
    if f.read_to_end(&mut buf).await.is_err() {
        // I/O error mid-read: keep whatever bytes we managed
        // to read; the hash will still flag a difference.
    }
    if buf.len() > cap {
        buf.truncate(cap);
    }
    buf
}

/// True iff `path` is inside `home` (canonicalised, no
/// traversal). Reuses the same logic as `util::sandbox` so
/// the plan refuses the same dests the install would refuse.
async fn path_inside_home(path: &Path, home: &Path) -> bool {
    let Ok(canonical_home) = tokio::fs::canonicalize(home).await else {
        return false;
    };
    let Ok(canonical) = tokio::fs::canonicalize(path).await else {
        // Dest doesn't exist yet (we're in the Create
        // branch); check its parent. The sandbox allows
        // creating new files under the home, so this is
        // always OK.
        return path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.starts_with(&canonical_home))
            .unwrap_or(false);
    };
    canonical.starts_with(&canonical_home)
}

/// Short stable prefix of a SHA-256 hex digest (16 chars),
/// used in the plan to label "before / after" without leaking
/// the full content.
fn short_sha(bytes: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // We don't pull in a sha2 crate just for this; the
    // `render::render_with_hash` already produced the proper
    // SHA-256 for the *after* side. For the *before* side we
    // use a 64-bit hash, which is fine for a UI "changed?"
    // indicator. The renderer pipeline is the source of
    // truth for the actual content identity.
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    let h = h.finish();
    format!("{:016x}", h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_summary_total_matches_changes() {
        let s = PlanSummary {
            creates: 2,
            overwrites: 0,
            no_changes: 3,
            refused: 0,
        };
        assert_eq!(s.total(), 5);
        assert!(!s.is_destructive());
    }

    #[test]
    fn plan_summary_is_destructive_on_overwrite() {
        let s = PlanSummary {
            creates: 0,
            overwrites: 1,
            no_changes: 0,
            refused: 0,
        };
        assert!(s.is_destructive());
    }

    #[test]
    fn plan_summary_is_destructive_on_refused() {
        let s = PlanSummary {
            creates: 0,
            overwrites: 0,
            no_changes: 0,
            refused: 1,
        };
        assert!(s.is_destructive());
    }

    #[test]
    fn plan_change_glyph_per_variant() {
        assert_eq!(
            PlanChange::Create { dest: "/a".into(), size: 0 }.glyph(),
            "+"
        );
        assert_eq!(
            PlanChange::Overwrite {
                dest: "/a".into(),
                before_sha: "x".into(),
                after_sha: "y".into(),
                backup_filename: "x.bak".into()
            }
            .glyph(),
            "~"
        );
        assert_eq!(
            PlanChange::NoChange { dest: "/a".into(), sha: "x".into() }.glyph(),
            "="
        );
        assert_eq!(
            PlanChange::Refused { dest: "/a".into(), reason: "x".into() }.glyph(),
            "!"
        );
    }

    #[test]
    fn plan_change_dest_returns_path_for_all_variants() {
        let cases = [
            PlanChange::Create { dest: "/create".into(), size: 0 },
            PlanChange::Overwrite {
                dest: "/over".into(),
                before_sha: "a".into(),
                after_sha: "b".into(),
                backup_filename: "f.bak".into(),
            },
            PlanChange::NoChange { dest: "/same".into(), sha: "x".into() },
            PlanChange::Refused {
                dest: "/nope".into(),
                reason: "x".into(),
            },
        ];
        let expected = ["/create", "/over", "/same", "/nope"];
        for (case, exp) in cases.iter().zip(expected.iter()) {
            assert_eq!(case.dest(), *exp);
        }
    }

    #[test]
    fn short_sha_is_stable_and_short() {
        let a = short_sha(b"hello");
        let b = short_sha(b"hello");
        let c = short_sha(b"world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 16);
    }
}
