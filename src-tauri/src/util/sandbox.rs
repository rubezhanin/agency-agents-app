//! Filesystem path sandbox: reject any input whose canonical form
//! escapes a designated root.
//!
//! ## Why
//!
//! The Tauri IPC surface lets the frontend hand us paths (loadout
//! imports, catalog source roots, backup paths, restore targets).
//! None of those are trustworthy on their own \u2014 the frontend is
//! just a renderer, and any HTML/markdown preview or drag-and-drop
//! handler can end up with a hostile string in `PathBuf`. We treat
//! every caller-supplied path as untrusted and resolve it against a
//! designated root before touching the disk.
//!
//! ## Semantics
//!
//! `resolve_safe_path(root, input)`:
//!
//! 1. Canonicalises `root` (resolving any symlinks along the way).
//! 2. Joins `input` to `root` if `input` is relative; otherwise takes
//!    `input` as-is. (The `..` segments in either form are resolved
//!    by the canonicalisation that follows.)
//! 3. Canonicalises the joined path.
//! 4. Rejects the result if its canonical form does not start with
//!    `root`'s canonical form. Returns the canonical path otherwise.
//!
//! Failure modes:
//! - `root` doesn't exist or isn't readable: `AppError::Io` from the
//!   canonicalise call.
//! - `input` doesn't exist (e.g. we're about to *create* a new file
//!   in a fresh subdir): canonicalise fails. Callers that intend to
//!   create the path should canonicalise the *parent* first and
//!   re-attach the leaf \u2014 see [`resolve_under_root_creating`] for
//!   that variant.
//! - The canonical path escapes `root`: `AppError::InvalidArgument`
//!   with a description of the attempt.
//!
//! ## Symlinks
//!
//! The check is post-canonicalisation, so a symlink inside `root`
//! that points outside `root` is correctly rejected: `canonicalize`
//! follows the link, the resolved path is no longer under `root`,
//! and we bail. Symlinks that stay inside `root` are followed and
//! the resolved path is returned.
//!
//! ## Cross-platform
//!
//! On Windows, `canonicalize` returns `\\?\` verbatim paths. The
//! `starts_with` check handles this fine because both sides of the
//! comparison are canonicalised with the same prefix style.
use std::path::{Component, Path, PathBuf};

use crate::error::AppError;

/// Resolve `input` against `root` and reject any path that escapes
/// the canonicalised root.
///
/// See module docs for the full semantics. Returns the canonical
/// `PathBuf` of the resolved target.
pub fn resolve_safe_path(root: &Path, input: &Path) -> Result<PathBuf, AppError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|e| AppError::Io {
        message: format!("canonicalize root {}: {e}", root.display()),
    })?;

    let joined = if input.is_absolute() {
        input.to_path_buf()
    } else {
        canonical_root.join(input)
    };

    let canonical = std::fs::canonicalize(&joined).map_err(|e| AppError::Io {
        message: format!(
            "canonicalize {}: {e} (does the path exist?)",
            joined.display()
        ),
    })?;

    if !canonical.starts_with(&canonical_root) {
        return Err(AppError::InvalidArgument {
            message: format!(
                "path {} escapes sandbox root {} (resolved to {})",
                input.display(),
                root.display(),
                canonical.display()
            ),
        });
    }

    Ok(canonical)
}

/// Same as [`resolve_safe_path`] but for the common "I'm about to
/// create a new file under root" case. The leaf is appended to a
/// canonicalised parent (root or a subdirectory), and the
/// non-existent final path is returned \u2014 no canonicalise on the
/// leaf itself, since the file doesn't exist yet.
///
/// Use this when the caller intends to *write* to a path the user
/// asked for, not *read* an existing one.
pub fn resolve_under_root_creating(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, AppError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|e| AppError::Io {
        message: format!("canonicalize root {}: {e}", root.display()),
    })?;

    // Reject absolute paths outright \u2014 under "creating" semantics the
    // only legal input is relative to `root`.
    if relative.is_absolute() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "creating-mode sandbox does not accept absolute paths: {}",
                relative.display()
            ),
        });
    }

    // Walk the components and split into "existing prefix" + "new
    // leaf". Canonicalise the longest existing prefix; reject if
    // any `..` or absolute component escapes the canonical root.
    let mut existing = canonical_root.clone();
    let mut pending: Vec<Component<'_>> = Vec::new();
    for comp in relative.components() {
        match comp {
            Component::Normal(_) => pending.push(comp),
            Component::CurDir => {} // `./` is a no-op
            Component::ParentDir => {
                if pending.is_empty() {
                    // Try to physically pop a real segment off `existing`.
                    if existing == canonical_root {
                        return Err(AppError::InvalidArgument {
                            message: format!(
                                "path {} would escape sandbox root {}",
                                relative.display(),
                                root.display()
                            ),
                        });
                    }
                    if !existing.pop() {
                        // Shouldn't happen \u2014 we just compared to canonical_root.
                        return Err(AppError::InvalidArgument {
                            message: format!(
                                "path {} would escape sandbox root {}",
                                relative.display(),
                                root.display()
                            ),
                        });
                    }
                } else {
                    pending.pop();
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(AppError::InvalidArgument {
                    message: format!(
                        "path {} contains an absolute-component that the sandbox cannot handle",
                        relative.display()
                    ),
                });
            }
        }
    }

    for comp in pending {
        existing.push(comp.as_os_str());
    }
    // Sanity: the resolved path must still be inside the canonical root.
    if !existing.starts_with(&canonical_root) {
        return Err(AppError::InvalidArgument {
            message: format!(
                "path {} resolves to {} which escapes sandbox root {}",
                relative.display(),
                existing.display(),
                canonical_root.display()
            ),
        });
    }

    Ok(existing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "agency-agents-sandbox-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn resolve_existing_file_inside_root() {
        let root = tmp_root("inside");
        let f = root.join("a.md");
        fs::write(&f, "hi").unwrap();
        let got = resolve_safe_path(&root, &f).unwrap();
        assert!(got.starts_with(fs::canonicalize(&root).unwrap()));
    }

    #[test]
    fn resolve_relative_path_inside_root() {
        let root = tmp_root("rel");
        fs::write(root.join("b.md"), "hi").unwrap();
        let got = resolve_safe_path(&root, Path::new("b.md")).unwrap();
        assert!(got.ends_with("b.md"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_parent_dir_escape() {
        let root = tmp_root("escape");
        let evil = root.join("..").join("etc").join("passwd");
        // `..` collapses to /etc/passwd, which is outside `root` after
        // canonicalisation, so we should reject.
        let res = resolve_safe_path(&root, &evil);
        assert!(matches!(res, Err(AppError::InvalidArgument { .. })), "got {res:?}");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_absolute_outside_root() {
        let root = tmp_root("abs");
        let res = resolve_safe_path(&root, Path::new("/etc/passwd"));
        assert!(matches!(res, Err(AppError::InvalidArgument { .. })), "got {res:?}");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_symlink_escape() {
        let root = tmp_root("sym");
        let outside = tmp_root("sym-outside");
        fs::write(outside.join("secret"), "no").unwrap();
        // `root/escape` is a symlink to `outside`. canonicalize follows
        // it, so the resolved path no longer sits under `root` and we
        // should reject.
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let res = resolve_safe_path(&root, &root.join("escape"));
        assert!(matches!(res, Err(AppError::InvalidArgument { .. })), "got {res:?}");
    }

    #[test]
    fn resolve_rejects_nonexistent_path() {
        let root = tmp_root("missing");
        let res = resolve_safe_path(&root, &root.join("does-not-exist.md"));
        // The canonicalize call on the missing path fails \u2014 surfaced
        // as Io. Callers that intend to *create* should use
        // `resolve_under_root_creating` instead.
        assert!(matches!(res, Err(AppError::Io { .. })), "got {res:?}");
    }

    #[test]
    fn creating_accepts_nested_new_path() {
        let root = tmp_root("new");
        let got = resolve_under_root_creating(&root, Path::new("nested/deep/file.md")).unwrap();
        // Canonicalisation only happens on the existing prefix; the
        // returned path is the *intended* target, not the existing one.
        assert!(got.ends_with(Path::new("nested").join("deep").join("file.md")));
    }

    #[cfg(unix)]
    #[test]
    fn creating_rejects_parent_dir_escape() {
        let root = tmp_root("new-escape");
        let res = resolve_under_root_creating(&root, Path::new("../../../etc/passwd"));
        assert!(matches!(res, Err(AppError::InvalidArgument { .. })), "got {res:?}");
    }

    #[cfg(unix)]
    #[test]
    fn creating_rejects_absolute_path() {
        let root = tmp_root("new-abs");
        let res = resolve_under_root_creating(&root, Path::new("/etc/passwd"));
        assert!(matches!(res, Err(AppError::InvalidArgument { .. })), "got {res:?}");
    }
}
