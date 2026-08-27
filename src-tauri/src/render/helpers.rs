//! Pure helpers shared by the dispatch / output / dests layers.
//!
//! - `sha256_hex` — canonical hash for the ledger + reconcile.
//! - `slugify` — port of `scripts/lib.sh#slugify`.
//! - `source_field` / `source_body` — non-YAML frontmatter readers,
//!   ported from `scripts/lib.sh` so we keep byte parity with the
//!   upstream `convert.sh`.
//! - `resolve_opencode_color` — map an agency-agents `color` to a
//!   hex OpenCode can render.
//! - `toml_escape` — TOML basic-string escape port.
//!
//! Everything here is deterministic + IO-free + easy to test.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::registry;

/// SHA-256, lowercase hex — the canonical hash for the ledger + reconcile.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Match `scripts/lib.sh#get_field`: return the first literal `field: value`
/// line between exact `---` fences. The shell helper does not parse YAML, so
/// quotes and other source spelling must be preserved for byte parity.
pub(crate) fn source_field<'a>(source: &'a str, field: &str) -> &'a str {
    let prefix = format!("{field}: ");
    let mut fences = 0;
    for line in source.lines() {
        if line == "---" {
            fences += 1;
            continue;
        }
        if fences == 1 {
            if let Some(value) = line.strip_prefix(&prefix) {
                return value;
            }
        } else if fences >= 2 {
            break;
        }
    }
    ""
}

/// Match `body="$(get_body "$file")"` from the upstream converter. `awk`
/// emits one newline per body line and command substitution strips every
/// trailing newline before the heredoc adds exactly one back.
pub(crate) fn source_body(source: &str) -> String {
    let mut fences = 0;
    let mut body = String::new();
    for line in source.lines() {
        if line == "---" {
            fences += 1;
            continue;
        }
        if fences >= 2 {
            body.push_str(line);
            body.push('\n');
        }
    }
    while body.ends_with('\n') {
        body.pop();
    }
    body
}

/// Match `scripts/lib.sh#slugify`. Lowercases, replaces non-alphanumerics
/// with `-`, collapses runs, trims trailing dashes. Empty input → empty
/// output.
pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            previous_dash = false;
        } else if !out.is_empty() && !previous_dash {
            out.push('-');
            previous_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Map an agency-agents `color` (named or hex) to an OpenCode-safe `#RRGGBB`
/// (uppercase). Unknown → neutral gray. Ported from `resolve_opencode_color`.
pub(crate) fn resolve_opencode_color(color: &str) -> String {
    let c = color.trim().to_ascii_lowercase();
    let mapped = match c.as_str() {
        "cyan" => "#00FFFF",
        "blue" => "#3498DB",
        "green" => "#2ECC71",
        "red" => "#E74C3C",
        "purple" => "#9B59B6",
        "orange" => "#F39C12",
        "teal" => "#008080",
        "indigo" => "#6366F1",
        "pink" => "#E84393",
        "gold" => "#EAB308",
        "amber" => "#F59E0B",
        "neon-green" => "#10B981",
        "neon-cyan" => "#06B6D4",
        "metallic-blue" => "#3B82F6",
        "yellow" => "#EAB308",
        "violet" => "#8B5CF6",
        "rose" => "#F43F5E",
        "lime" => "#84CC16",
        "gray" => "#6B7280",
        "fuchsia" => "#D946EF",
        other => other,
    };
    let hex = mapped.strip_prefix('#').unwrap_or(mapped);
    let is_hex6 = hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hex6 {
        format!("#{}", hex.to_ascii_uppercase())
    } else {
        "#6B7280".to_string()
    }
}

/// Escape a value for a TOML basic string (ported from `toml_escape_string`).
pub(crate) fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7F => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Shared "this tool has no app renderer" error.
pub(crate) fn unsupported(tool: &str) -> AppError {
    // Error messages use the kebab id (matching `scripts/install.sh`); fall back
    // to the raw id for an unrecognized tool.
    let kebab = registry::get(tool)
        .map(|m| m.kebab.as_str())
        .unwrap_or(tool);
    AppError::Io {
        message: format!("tool '{kebab}' is not supported for install yet (multi-file format)"),
    }
}

// `Path` is not used directly but importing it keeps the file's API
// surface consistent with the other `render::*` modules that consume
// `Path`. The re-export silences an "unused import" lint without
// disturbing callers.
#[allow(dead_code)]
pub(crate) fn _path_marker(_: &Path) {}
