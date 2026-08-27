//! Registry dispatch — surface-level helpers that read the tool's
//! `Tool` metadata and answer "can this tool deploy at scope X?",
//! "what's its human label?", and "where should an install land
//! scope-wise?".
//!
//! All four are pure functions of `(tool id, optional project_root)` —
//! no IO, no global state. Cheap to call from anywhere.

use std::path::Path;

use crate::registry;
use crate::types::Scope;

/// Whether `tool` can deploy USER-GLOBALLY (`~/…`). Most CLIs read a user-level
/// agents dir; Cursor is the exception — its global rules live in the Settings
/// UI, with no file path, so it's project-only. Sourced from the registry's
/// `scope` caps. Unknown ids → false.
pub fn supports_user(tool: &str) -> bool {
    registry::get(tool).is_some_and(|m| m.supports_user())
}

/// Whether `tool` can deploy into a SPECIFIC PROJECT (`<project>/…`). Sourced
/// from the registry's `scope` caps. Unknown ids → false.
pub fn supports_project(tool: &str) -> bool {
    registry::get(tool).is_some_and(|m| m.supports_project())
}

/// The scope an install lands in, derived from whether a project root was
/// chosen — NOT a fixed property of the tool. A project path ⇒ project scope.
pub fn scope_for(project_root: Option<&Path>) -> Scope {
    if project_root.is_some() {
        Scope::Project
    } else {
        Scope::User
    }
}

/// Human label for the UI, from the registry. Falls back to the raw id for an
/// unknown tool so callers always get a printable string.
pub fn label(tool: &str) -> String {
    registry::get(tool)
        .map(|m| m.label.clone())
        .unwrap_or_else(|| tool.to_string())
}
