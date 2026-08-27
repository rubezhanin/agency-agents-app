//! Destination-path resolution — `dests`.
//!
//! Picks the scope-appropriate template array (USER vs PROJECT) and
//! the matching root (`home` vs `project_root`), then substitutes
//! `{slug}` into every template. Most tools write a single file;
//! Copilot dual-writes to `~/.github` and `~/.copilot`.

use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::registry;

use super::helpers::unsupported;

/// Absolute destination path(s) for an installed agent.
///
/// `home` is the user's home dir (user-scoped tools). `project_root` is required
/// for project-scoped tools (cursor, opencode) and ignored otherwise.
pub fn dests(
    tool: &str,
    slug: &str,
    home: &Path,
    project_root: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    // A tool with no `dest` templates (and no renderer) is recognized-only.
    let dest = registry::get(tool)
        .and_then(|m| m.dest.as_ref())
        .ok_or_else(|| unsupported(tool))?;

    // Pick the scope-appropriate template array + its root. USER paths are rooted
    // at `$HOME`; PROJECT paths at the project root. Dual-scope same-path tools
    // (claude/codex/gemini/qwen) just re-root the identical relative template;
    // tools whose user/project dirs differ (opencode, copilot) carry separate
    // arrays in the JSON, so this picks the right one.
    let (templates, root): (&[String], &Path) = match project_root {
        Some(p) => (&dest.project, p),
        None => (&dest.user, home),
    };

    // An empty array for the requested scope means this tool can't deploy there.
    // The only such case today is Cursor: project-only, so a user-scoped (no
    // project root) request must surface the existing "project path required"
    // error rather than a multi-file `unsupported`.
    if templates.is_empty() {
        let kebab = registry::get(tool)
            .map(|m| m.kebab.as_str())
            .unwrap_or(tool);
        return Err(AppError::Io {
            message: format!("tool '{kebab}' is project-scoped; a project path is required"),
        });
    }

    Ok(templates
        .iter()
        .map(|t| join_template(root, t, slug))
        .collect())
}

/// Build a `PathBuf` from a destination template by joining each `/`-separated
/// component onto `root` individually, so the resulting path uses the
/// platform-native separator (e.g. `\` on Windows). A single
/// `root.join(".codex/agents/foo.toml")` would embed the `/` as a literal
/// character in the final component, producing a path that mismatches what
/// `Path::to_string_lossy()` returns on `Path::join` chains in tests.
fn join_template(root: &Path, template: &str, slug: &str) -> PathBuf {
    let expanded = template.replace("{slug}", slug);
    let mut out = root.to_path_buf();
    for comp in expanded.split(['/', '\\']) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        out.push(comp);
    }
    out
}
