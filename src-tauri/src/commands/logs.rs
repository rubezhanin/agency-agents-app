//! Tauri commands for the structured-log surface (0.4.7).
//!
//! The file layer in `lib.rs::setup` writes daily-rolled JSON lines
//! into `<app_data>/logs/app.YYYY-MM-DD.json` via `tracing-appender`.
//! These three commands are the read path for the Settings → Logs UI:
//!
//! - `logs_list` — every log file in the directory, newest first.
//! - `logs_read(name)` — tail the named file (or the most recent
//!   `tail_bytes` bytes if the file is huge, to keep the IPC payload
//!   and the frontend's render loop cheap).
//! - `logs_clear` — delete every log file in the directory. Useful for
//!   the "I want to ship a debug report" flow: wipe, reproduce, send.
//!
//! All three share `logs_dir(app)`, which mirrors `corpus::app_data_dir`
//! + `/logs` and is the only place on disk these commands look. Any
//! file in that directory we can't parse is silently skipped (we
//! don't want a stray `app.json.1234.tmp` from `tracing-appender`'s
//! mid-rotation atomic-rename to make the whole list call fail).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Manager};

use crate::error::AppError;
use crate::types::LogFile;

const LOG_PREFIX: &str = "app.";
const LOG_SUFFIX: &str = ".json";
/// Cap on the bytes we hand back to the frontend for a single file.
/// 256 KB is enough to see "what happened in the last few minutes"
/// without freezing the renderer on a multi-day stale file.
const TAIL_BYTES: u64 = 256 * 1024;

fn logs_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    let adir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal {
            message: format!("resolve app_data_dir: {e}"),
        })?;
    Ok(adir.join("logs"))
}

/// List every parseable log file in `<app_data>/logs/`, newest first.
/// The directory may not exist yet (fresh install) — we return an
/// empty list rather than an error in that case.
#[tauri::command]
pub async fn logs_list(app: AppHandle) -> Result<Vec<LogFile>, AppError> {
    let dir = logs_dir(&app)?;
    let mut out: Vec<LogFile> = Vec::new();
    let read = match tokio::fs::read_dir(&dir).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => {
            return Err(AppError::Io {
                message: format!("read logs dir {}: {e}", dir.display()),
            })
        }
    };
    let mut entries = read;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Io {
            message: format!("iterate logs dir: {e}"),
        })?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_log_filename(&name) {
            continue;
        }
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue, // skip the unreadable ones
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        out.push(LogFile {
            name,
            size: meta.len(),
            created_at: modified,
        });
    }
    // Newest first — `modified` is RFC3339, lex order matches chrono order.
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

/// Read at most `TAIL_BYTES` from the end of the named log file. The
/// frontend doesn't need to render megabytes; the tail is what the
/// user is debugging right now.
#[tauri::command]
pub async fn logs_read(app: AppHandle, name: String) -> Result<String, AppError> {
    let dir = logs_dir(&app)?;
    if !is_log_filename(&name) {
        return Err(AppError::InvalidArgument {
            message: format!("not a log filename: {name}"),
        });
    }
    let path = dir.join(&name);
    // Resolve symlinks and confirm the resolved path is still inside
    // `logs_dir` — defense in depth, in case a hand-edited caller
    // passes `../something-else` and we later relax the filename check.
    let canonical_logs = tokio::fs::canonicalize(&dir).await.map_err(|e| AppError::Io {
        message: format!("canonicalize logs dir: {e}"),
    })?;
    let canonical = tokio::fs::canonicalize(&path).await.map_err(|e| AppError::Io {
        message: format!("open log file {}: {e}", path.display()),
    })?;
    if !canonical.starts_with(&canonical_logs) {
        return Err(AppError::InvalidArgument {
            message: format!(
                "log file {} escapes logs dir",
                canonical.display()
            ),
        });
    }
    read_tail(&canonical, TAIL_BYTES).await
}

/// Delete every log file in `<app_data>/logs/`. We don't try to be
/// clever about "keep the current one" — the user pressed Clear, they
/// want a clean slate. Returns the number of files removed.
#[tauri::command]
pub async fn logs_clear(app: AppHandle) -> Result<u32, AppError> {
    let dir = logs_dir(&app)?;
    let mut removed: u32 = 0;
    let read = match tokio::fs::read_dir(&dir).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(AppError::Io {
                message: format!("read logs dir {}: {e}", dir.display()),
            })
        }
    };
    let mut entries = read;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Io {
            message: format!("iterate logs dir: {e}"),
        })?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_log_filename(&name) {
            continue;
        }
        if entry.metadata().await.map(|m| m.is_file()).unwrap_or(false)
            && tokio::fs::remove_file(entry.path()).await.is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Absolute path of the logs dir, for the Settings → Logs "open
/// folder" button (paired with `reveal_path`).
#[tauri::command]
pub async fn logs_folder_path(app: AppHandle) -> Result<String, AppError> {
    Ok(logs_dir(&app)?.to_string_lossy().to_string())
}

// ----- helpers -----

fn is_log_filename(name: &str) -> bool {
    name.starts_with(LOG_PREFIX) && name.ends_with(LOG_SUFFIX)
}

async fn read_tail(path: &Path, max: u64) -> Result<String, AppError> {
    let meta = tokio::fs::metadata(path).await.map_err(|e| AppError::Io {
        message: format!("stat {}: {e}", path.display()),
    })?;
    let len = meta.len();
    let start = len.saturating_sub(max);
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut f = tokio::fs::File::open(path).await.map_err(|e| AppError::Io {
        message: format!("open {}: {e}", path.display()),
    })?;
    if start > 0 {
        f.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|e| AppError::Io {
                message: format!("seek {}: {e}", path.display()),
            })?;
    }
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf)
        .await
        .map_err(|e| AppError::Io {
            message: format!("read {}: {e}", path.display()),
        })?;
    // Drop the first partial line if we sliced into the middle of one.
    let s = String::from_utf8_lossy(&buf);
    if start > 0 {
        if let Some(nl) = s.find('\n') {
            return Ok(s[nl + 1..].to_string());
        }
    }
    Ok(s.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_log_filename_accepts_app_dated_json() {
        assert!(is_log_filename("app.2026-08-29.json"));
        assert!(is_log_filename("app.json")); // today's file (rotation boundary)
        assert!(!is_log_filename("notes.md"));
        assert!(!is_log_filename("../app.2026-08-29.json"));
    }
}
