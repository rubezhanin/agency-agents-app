//! Corpus tarball parsing + safe extraction.
//!
//! Shared by the live "Refresh" button (which fetches the codeload tarball)
//! and the managed-catalog provisioning path (which drops the same tarball
//! into `~/.agency-agents` when no `git` binary is on PATH). Also re-hosts
//! the `parse_agent_dirs` helper that reads `AGENT_DIRS=(...)` out of
//! `scripts/convert.sh`, so a freshly-added upstream division is picked up
//! automatically.
//!
//! Path-traversal safe: we only ever join the *sanitized* category + file
//! name onto the destination; the raw archive path is never used to build a
//! write target. The codeload tarball nests everything under a single
//! `agency-agents-main/` top-level dir, which we strip.

use std::path::Path;

use crate::error::AppError;

/// Parse the bash-style `AGENT_DIRS=(...)` array from `scripts/convert.sh`.
/// Used to discover the live category set from a tarball refresh.
pub(crate) fn parse_agent_dirs(script: &str) -> Option<Vec<String>> {
    let start = script.find("AGENT_DIRS=(")?;
    let after = &script[start + "AGENT_DIRS=(".len()..];
    let end = after.find(')')?;
    let body = &after[..end];

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw_line in body.lines() {
        // Strip an inline comment, then split on whitespace.
        let line = raw_line.split('#').next().unwrap_or("");
        for tok in line.split_whitespace() {
            // Defensive: ignore anything that isn't a plausible dir slug.
            if tok.is_empty()
                || !tok
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                continue;
            }
            if seen.insert(tok.to_string()) {
                out.push(tok.to_string());
            }
        }
    }
    Some(out)
}

/// Gunzip the tarball and decode it to raw `tar` bytes, capped against a gzip
/// bomb. Shared by `extract_categories` and `categories_from_tarball`.
pub(crate) fn gunzip_capped(tar_gz: &[u8]) -> Result<Vec<u8>, AppError> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(tar_gz);
    let mut capped = gz.take(super::MAX_TARBALL_BYTES * 8);
    let mut tar_bytes = Vec::new();
    capped
        .read_to_end(&mut tar_bytes)
        .map_err(|e| AppError::Io {
            message: format!("gunzip corpus tarball: {e}"),
        })?;
    Ok(tar_bytes)
}

/// Read `scripts/convert.sh` out of the tarball and parse its `AGENT_DIRS`
/// array, so a refresh adopts upstream's current division set. `None` if the
/// script isn't present or doesn't parse (caller falls back to the default).
pub(crate) fn categories_from_tarball(tar_gz: &[u8]) -> Option<Vec<String>> {
    use std::io::Read;
    let tar_bytes = gunzip_capped(tar_gz).ok()?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    for entry in archive.entries().ok()? {
        let mut entry = entry.ok()?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().ok()?;
        let comps: Vec<String> = path
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => s.to_str().map(|s| s.to_string()),
                _ => None,
            })
            .collect();
        // top/scripts/convert.sh
        if comps.len() == 3 && comps[1] == "scripts" && comps[2] == "convert.sh" {
            let mut text = String::new();
            entry.read_to_string(&mut text).ok()?;
            return parse_agent_dirs(&text).filter(|v| !v.is_empty());
        }
    }
    None
}

/// Gunzip + untar `tar_gz`, writing every `<category>/<slug>.md` whose category
/// is in `categories` into `<dest>/<category>/`, plus `scripts/convert.sh` (so
/// the working copy stays self-describing). The codeload tarball nests
/// everything under a single `agency-agents-main/` top-level dir, which we
/// strip. Returns the count of agent files written.
///
/// Path-traversal safe: we only ever join the *sanitized* `category` +
/// `file_name` onto `dest`; the raw archive path is never used to build a
/// write target.
pub(crate) fn extract_categories(
    tar_gz: &[u8],
    dest: &Path,
    categories: &[String],
) -> Result<u32, AppError> {
    use std::io::Read;

    let tar_bytes = gunzip_capped(tar_gz)?;
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    let entries = archive.entries().map_err(|e| AppError::Io {
        message: format!("read tar entries: {e}"),
    })?;

    let is_category = |c: &str| categories.iter().any(|cat| cat == c);
    let mut written = 0u32;
    for entry in entries {
        let mut entry = entry.map_err(|e| AppError::Io {
            message: format!("tar entry: {e}"),
        })?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|e| AppError::Io {
            message: format!("tar entry path: {e}"),
        })?;
        // Strip the single top-level `agency-agents-main/` component.
        let comps: Vec<String> = path
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => s.to_str().map(|s| s.to_string()),
                _ => None,
            })
            .collect();

        // Persist the tooling so subsequent launches re-derive categories.
        if comps.len() == 3 && comps[1] == "scripts" && comps[2] == "convert.sh" {
            let scripts_dir = dest.join("scripts");
            let _ = std::fs::create_dir_all(&scripts_dir);
            let mut buf = Vec::new();
            if entry.read_to_end(&mut buf).is_ok() {
                let _ = std::fs::write(scripts_dir.join("convert.sh"), &buf);
            }
            continue;
        }

        if comps.len() < 3 {
            continue; // need top/<category>/<file>
        }
        let category = comps[1].as_str();
        let fname = comps.last().unwrap().as_str();
        if !is_category(category) {
            continue;
        }
        if !fname.ends_with(".md") || fname == "README.md" {
            continue;
        }
        // Sanitized target — built only from validated components.
        let cat_dir = dest.join(category);
        std::fs::create_dir_all(&cat_dir).map_err(|e| AppError::Io {
            message: format!("create {}: {e}", cat_dir.display()),
        })?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| AppError::Io {
            message: format!("read tar file {}: {e}", fname),
        })?;
        std::fs::write(cat_dir.join(fname), &buf).map_err(|e| AppError::Io {
            message: format!("write {}: {e}", cat_dir.join(fname).display()),
        })?;
        written += 1;
    }
    Ok(written)
}
