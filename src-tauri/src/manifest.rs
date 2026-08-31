//! Upstream tool manifest — strongly-typed schema for the
//! `tools.json` that lives at the repo root. This is the
//! "single source of truth" for *what tools exist*; the
//! app-specific question of *which ones we can install* is
//! derived from the `format` field (a tool is installable iff we
//! ship a native Rust renderer for its `format`).
//!
//! ## Why a new module
//!
//! `registry.rs` (the existing module) holds the *deserialised*
//! view of the bundled `data/tools.json`, but with a deliberately
//! loose type — every field except `id` is optional, no
//! invariants are checked, and there's no "manifest" semantics:
//! just a hashmap of properties. This is fine for the
//! *app's* consumption (it can be defensive about missing
//! fields) but it doesn't help when the *upstream catalog* wants
//! to ship a typed schema and ask the app to consume it as such.
//!
//! `manifest` is the next-generation model: a strict schema,
//! validation at load time, and a separate IPC surface so the
//! Settings → Catalog pane can show "your local manifest is X
//! commits behind upstream" or similar diagnostics later. It's
//! intentionally additive — `registry.rs` still works as-is
//! and Phase 3 (plugin architecture) will be the right time to
//! fold the two together.
//!
//! ## What's validated
//!
//! The validator enforces the minimum invariants the manifest
//! schema implies:
//!
//! 1. Every entry has a non-empty `id`, `kebab`, `label`, `format`,
//!    `install_kind`.
//! 2. `id` and `kebab` are unique across entries.
//! 3. `format` is one of the values the app's renderer can
//!    consume (the `IMPLEMENTED_FORMATS` from `registry.rs`,
//!    re-exported here so the validator is local). The manifest
//!    may *contain* other formats (the catalog owns them), but
//!    the app can only flag those as "recognized-only".
//! 4. `install_kind` is one of `per_agent` | `roster` | `plugin`.
//! 5. `dest.user` and `dest.project` are arrays of strings,
//!    each containing the `{slug}` placeholder.
//! 6. `version.bin` is non-empty when present.
//!
//! Anything else is reported as a warning, not an error — the
//! manifest is still loadable.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AppError;

/// Validated tool manifest. Loaded once at startup (or on
/// Settings → Catalog → "Refresh manifest" click) and re-used.
/// Mutation is rare enough that we don't bother with a
/// `Mutex` / `RwLock` — readers either see the old or the new.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct ToolManifest {
    /// Map of `kebab` → entry. We key on `kebab` (not `id`)
    /// because `kebab` is the on-disk / catalog-stable key
    /// (e.g. `claude-code`); `id` is the camelCase wire value
    /// (`claudeCode`) the frontend uses.
    pub tools: HashMap<String, ToolEntry>,
    /// Loader-level warnings: non-fatal but worth surfacing
    /// (e.g. an unknown `format` that the app can't render).
    /// Populated by the validator, not the deserialiser.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// One tool entry from the catalog. The schema is **strict** —
/// every required field must be present, otherwise loading
/// fails with a typed error. This is a deliberate change from
/// the loose `registry::ToolSpec`, where most fields were
/// optional.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct ToolEntry {
    /// camelCase wire value (e.g. `claudeCode`). Must be unique
    /// across the manifest.
    pub id: String,
    /// Human label shown in the UI (e.g. "Claude Code").
    pub label: String,
    /// Short label for the sidebar / dense layouts.
    pub short: String,
    /// Stable on-disk key (e.g. `claude-code`). Must be
    /// unique; this is the dict key in `tools.json`.
    pub kebab: String,
    /// Brand colour (hex). Optional — older entries may omit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Icon name (resolved by the frontend icon map).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Sort order in the Tools panel.
    pub order: u32,
    /// Where the tool can deploy.
    pub scope: ScopeCaps,
    /// Detection hints.
    pub detect: Detect,
    /// Probe command for the local version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<VersionProbe>,
    /// Renderer contract. The same `format` name guarantees
    /// byte-identical output across tools.
    pub format: String,
    /// How the slug is derived for this tool. `name` = use
    /// the agent's `name` frontmatter, `source` = use the
    /// file basename, `null` (omitted) = no per-agent slug
    /// (roster / plugin). Stored as a raw string and validated
    /// post-parse so the bundled `tools.json` (which uses
    /// kebab values like `per-agent`) doesn't have to be
    /// rewritten for this spike. Phase 3 (plugin architecture)
    /// will decide which form is canonical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug_from: Option<String>,
    /// Optional prefix on the slug (e.g. `agency-` for
    /// `osaurus` so the dir is `~/.osaurus/skills/agency-foo/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug_prefix: Option<String>,
    /// Destination patterns, one per supported scope.
    pub dest: DestPatterns,
    /// Install mechanism. Stored as a raw string and validated
    /// post-parse; see `KNOWN_INSTALL_KINDS` and the
    /// `validator()` pass.
    pub install_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct ScopeCaps {
    #[serde(default)]
    pub user: bool,
    #[serde(default)]
    pub project: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct Detect {
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct VersionProbe {
    pub bin: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
pub struct DestPatterns {
    #[serde(default)]
    pub user: Vec<String>,
    #[serde(default)]
    pub project: Vec<String>,
}

// ---------- Loading + validation ----------

/// Errors that surface as IPC failures or startup panics. The
/// `msg` field is what the user sees; the rest is for the
/// developer / log line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestError {
    Io { msg: String },
    Json { msg: String },
    Schema { msg: String },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { msg } => write!(f, "manifest io: {msg}"),
            Self::Json { msg } => write!(f, "manifest json: {msg}"),
            Self::Schema { msg } => write!(f, "manifest schema: {msg}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<ManifestError> for AppError {
    fn from(e: ManifestError) -> AppError {
        AppError::Io {
            message: e.to_string(),
        }
    }
}

/// Render formats we ship native Rust renderers for. Same
/// list as `registry::IMPLEMENTED_FORMATS`; duplicated here so
/// the validator is self-contained. Phase 3 will fold the two
/// into a single source.
pub const IMPLEMENTED_FORMATS: &[&str] = &[
    "identity",
    "codex-toml",
    "gemini-md",
    "qwen-md",
    "zcode-md",
    "cursor-mdc",
    "opencode-md",
    "skill-md",
    "hermes-router-plugin",
];

/// `installKind` values we accept. Strings outside this set
/// are a schema error (the catalog shouldn't ship unknown
/// kinds without a coordinated app release). We accept both
/// the kebab form (`per-agent`, used by the bundled
/// `tools.json`) and the camelCase form (`perAgent`, what ts-rs
/// will produce from the `InstallKind` enum once Phase 3
/// folds the two together).
const KNOWN_INSTALL_KINDS: &[&str] = &[
    "per_agent",
    "perAgent",
    "per-agent",
    "roster",
    "plugin",
];

/// Load a manifest from a path on disk. The path is typically
/// the bundled `data/tools.json`; Phase 3 will also point this
/// at an upstream-fetched manifest.
#[allow(dead_code)] // unused in this spike — wired in Phase 3 IPC
pub fn load_manifest(path: &Path) -> Result<ToolManifest, ManifestError> {
    let bytes = std::fs::read(path).map_err(|e| ManifestError::Io {
        msg: format!("read manifest {}: {e}", path.display()),
    })?;
    parse_manifest(&bytes)
}

/// Parse a manifest from raw bytes. `bytes` is expected to be
/// the JSON file at the repo root: `{"tools": { "kebab": {...},
/// ...}}`. The top-level `_note` field (a human-readable
/// comment in the upstream file) is ignored.
pub fn parse_manifest(bytes: &[u8]) -> Result<ToolManifest, ManifestError> {
    // The upstream file has a top-level `_note` we don't want
    // to validate; deserialize into a permissive shape first,
    // then map into the strict `ToolEntry` per tool.
    //
    // We can't use `#[serde(flatten)]` on a newtype wrapper
    // around `ToolEntry` because serde's flatten support
    // requires a *struct* with named fields, not a tuple
    // struct. Instead we deserialize the same JSON twice —
    // once into `serde_json::Value` to validate it's an
    // object, and once into `ToolEntry` for the strict
    // fields — using a small helper closure.
    #[derive(Deserialize)]
    struct RawManifest {
        tools: HashMap<String, serde_json::Value>,
    }

    let raw: RawManifest = serde_json::from_slice(bytes).map_err(|e| ManifestError::Json {
        msg: format!("parse manifest: {e}"),
    })?;

    let mut manifest = ToolManifest {
        tools: HashMap::with_capacity(raw.tools.len()),
        warnings: Vec::new(),
    };
    for (kebab, value) in raw.tools {
        let entry: ToolEntry = serde_json::from_value(value).map_err(|e| ManifestError::Json {
            msg: format!("entry {kebab:?}: {e}"),
        })?;
        if entry.kebab != kebab {
            return Err(ManifestError::Schema {
                msg: format!(
                    "manifest key {kebab:?} does not match entry.kebab {:?} — keys must equal kebab",
                    entry.kebab
                ),
            });
        }
        manifest.tools.insert(kebab, entry);
    }
    validate(&mut manifest);
    Ok(manifest)
}

/// Run the schema invariants. Mutates `manifest.warnings` in
/// place. Hard schema errors are not raised here — those are
/// caught by serde at parse time (e.g. missing required field).
/// This is the "soft" pass: missing-but-allowed checks, plus the
/// cross-entry uniqueness invariants.
pub fn validate(manifest: &mut ToolManifest) {
    let mut seen_ids: HashSet<&str> = HashSet::new();
    let mut seen_kebabs: HashSet<&str> = HashSet::new();
    let implemented: HashSet<&str> = IMPLEMENTED_FORMATS.iter().copied().collect();

    for (kebab, entry) in manifest.tools.iter() {
        // Empty / missing required field — serde should have
        // caught this; if we see it here, the upstream file
        // is corrupt and we flag it.
        if entry.id.is_empty() {
            manifest
                .warnings
                .push(format!("entry {kebab:?} has empty id"));
        }
        if entry.label.is_empty() {
            manifest
                .warnings
                .push(format!("entry {kebab:?} has empty label"));
        }
        if entry.kebab.is_empty() {
            manifest
                .warnings
                .push(format!("entry {kebab:?} has empty kebab"));
        }
        if entry.format.is_empty() {
            manifest
                .warnings
                .push(format!("entry {kebab:?} has empty format"));
        }

        if !seen_ids.insert(entry.id.as_str()) {
            manifest
                .warnings
                .push(format!("duplicate id {:?} (also seen on another entry)", entry.id));
        }
        if !seen_kebabs.insert(entry.kebab.as_str()) {
            manifest
                .warnings
                .push(format!("duplicate kebab {:?} (also seen on another entry)", entry.kebab));
        }

        if !implemented.contains(entry.format.as_str()) {
            manifest.warnings.push(format!(
                "entry {kebab:?} has format {:?} which the app does not implement; \
                 it will appear as recognized-only in the Tools panel",
                entry.format
            ));
        }

        if !KNOWN_INSTALL_KINDS.contains(&entry.install_kind.as_str()) {
            manifest.warnings.push(format!(
                "entry {kebab:?} has unknown installKind {:?} (expected one of: per_agent, perAgent, roster, plugin)",
                entry.install_kind
            ));
        }

        // `{slug}` placeholder is required for per-agent tools
        // (they materialise per-agent files). Plugin and roster
        // installs are NOT per-agent renderable, so the absence
        // of `{slug}` is expected there.
        let needs_slug_placeholder = entry.install_kind == "per_agent"
            || entry.install_kind == "perAgent"
            || entry.install_kind == "per-agent";
        for pattern in entry.dest.user.iter().chain(entry.dest.project.iter()) {
            if !pattern.contains("{slug}") && needs_slug_placeholder {
                manifest.warnings.push(format!(
                    "entry {kebab:?} dest pattern {pattern:?} does not contain {{slug}}"
                ));
            }
        }

        if let Some(v) = &entry.version {
            if v.bin.is_empty() {
                manifest.warnings.push(format!(
                    "entry {kebab:?} has version probe with empty bin"
                ));
            }
        }
    }
}

// `install_kind` is a raw `String` (validated in `validator`),
// so the helpers from the earlier enum version are gone.

// ---------- Bundle access ----------

/// The bundled baseline manifest. We use `include_str!` so the
/// file is baked into the binary at compile time — no
/// filesystem lookup at startup, and no risk of a missing /
/// partially-synced file on a fresh install.
const BUNDLED_MANIFEST: &str = include_str!("../data/tools.json");

/// Load the bundled manifest. The same path is used by every
/// fresh install until Phase 3 wires an upstream fetch.
#[allow(dead_code)] // unused in this spike — wired in Phase 3 IPC
pub fn bundled_manifest() -> Result<ToolManifest, ManifestError> {
    parse_manifest(BUNDLED_MANIFEST.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-tool fixture exercising the validator's
    /// happy-path cases. The bundled real `tools.json` is
    /// used in another test, so we don't accidentally cover
    /// the same ground twice.
    fn minimal_ok() -> &'static str {
        r##"{
            "tools": {
                "claude-code": {
                    "id": "claudeCode",
                    "label": "Claude Code",
                    "short": "Claude",
                    "kebab": "claude-code",
                    "accent": "#D97757",
                    "icon": "claudecode",
                    "order": 1,
                    "scope": {"user": true, "project": true},
                    "detect": {"dirs": [".claude"], "agentsDir": ".claude/agents"},
                    "version": {"bin": "claude", "args": ["--version"]},
                    "format": "identity",
                    "installKind": "perAgent",
                    "slugFrom": "source",
                    "dest": {
                        "user": [".claude/agents/{slug}.md"],
                        "project": [".claude/agents/{slug}.md"]
                    }
                },
                "hermes": {
                    "id": "hermes",
                    "label": "Hermes",
                    "short": "Hermes",
                    "kebab": "hermes",
                    "order": 14,
                    "scope": {"user": true, "project": false},
                    "detect": {"dirs": [".hermes"]},
                    "version": {"bin": "hermes", "args": ["--version"]},
                    "format": "hermes-router-plugin",
                    "installKind": "plugin",
                    "dest": {
                        "user": [".hermes/plugins/agency-agents-router"],
                        "project": []
                    }
                }
            }
        }"##
    }

    #[test]
    fn parse_minimal_ok() {
        let m = parse_manifest(minimal_ok().as_bytes()).expect("parse");
        assert_eq!(m.tools.len(), 2);
        assert!(m.tools.contains_key("claude-code"));
        assert!(m.tools.contains_key("hermes"));
        // Validator runs as part of parse_manifest.
        assert!(
            m.warnings.is_empty(),
            "no warnings expected for a clean fixture, got: {:?}",
            m.warnings
        );
    }

    #[test]
    fn parse_bundled_real_manifest() {
        // The bundled `data/tools.json` we ship. We expect the
        // manifest to parse cleanly. The file intentionally
        // contains tools with formats the app doesn't implement
        // (Aider, Windsurf, OpenClaw, Kimi) — these are the
        // "recognised-only" tools the README lists as deferred
        // for v1.x. The validator must surface them as
        // *warnings*, not errors, so the Settings → Tools pane
        // can show them as dimmed.
        let m = bundled_manifest().expect("bundled manifest");
        // 15 tools as of v0.4.7.
        assert!(m.tools.len() >= 14, "expected at least 14 tools, got {}", m.tools.len());
        // Recognised-only formats expected to appear as warnings
        // (these are the tools the README documents as
        // recognised-only, deferred to v1.x).
        let recognised_only = [
            "aider-conventions",
            "windsurf-rules",
            "openclaw-workspace",
            "kimi-agent",
        ];
        for fmt in recognised_only {
            let present = m.warnings.iter().any(|w| w.contains(fmt));
            assert!(
                present,
                "expected recognised-only warning for {fmt:?}, warnings: {:?}",
                m.warnings
            );
        }
        // No unexpected warnings — i.e. no format the bundled
        // file claims to support that we don't know about.
        for w in &m.warnings {
            let known_recognised = recognised_only.iter().any(|r| w.contains(r));
            assert!(
                known_recognised,
                "bundled manifest has unexpected warning: {w}"
            );
        }
    }

    #[test]
    fn unknown_format_produces_warning() {
        let json = r#"{
            "tools": {
                "future": {
                    "id": "future", "label": "Future", "short": "F",
                    "kebab": "future", "order": 99,
                    "scope": {"user": true, "project": false},
                    "detect": {"dirs": []},
                    "format": "not-yet-implemented",
                    "installKind": "perAgent",
                    "dest": {"user": [".future/{slug}"], "project": []}
                }
            }
        }"#;
        let m = parse_manifest(json.as_bytes()).expect("parse");
        assert_eq!(m.warnings.len(), 1);
        assert!(m.warnings[0].contains("not-yet-implemented"));
    }

    #[test]
    fn dest_pattern_without_slug_placeholder_warns() {
        let json = r#"{
            "tools": {
                "bad-dest": {
                    "id": "badDest", "label": "Bad", "short": "B",
                    "kebab": "bad-dest", "order": 0,
                    "scope": {"user": true, "project": false},
                    "detect": {"dirs": []},
                    "format": "identity",
                    "installKind": "perAgent",
                    "dest": {"user": [".nope/static-name.md"], "project": []}
                }
            }
        }"#;
        let m = parse_manifest(json.as_bytes()).expect("parse");
        assert!(
            m.warnings.iter().any(|w| w.contains("{slug}")),
            "expected {{slug}} warning, got: {:?}",
            m.warnings
        );
    }

    #[test]
    fn unknown_install_kind_warns() {
        let json = r#"{
            "tools": {
                "weird": {
                    "id": "weird", "label": "Weird", "short": "W",
                    "kebab": "weird", "order": 0,
                    "scope": {"user": true, "project": false},
                    "detect": {"dirs": []},
                    "format": "identity",
                    "installKind": "magic",
                    "dest": {"user": [".weird/{slug}"], "project": []}
                }
            }
        }"#;
        let m = parse_manifest(json.as_bytes()).expect("parse");
        assert!(
            m.warnings.iter().any(|w| w.contains("magic")),
            "expected unknown-installKind warning"
        );
    }

    #[test]
    fn duplicate_ids_and_kebabs_warn() {
        let json = r#"{
            "tools": {
                "a": {
                    "id": "shared", "label": "A", "short": "A",
                    "kebab": "a", "order": 0,
                    "scope": {"user": true, "project": false},
                    "detect": {"dirs": []},
                    "format": "identity", "installKind": "perAgent",
                    "dest": {"user": [".a/{slug}"], "project": []}
                },
                "b": {
                    "id": "shared", "label": "B", "short": "B",
                    "kebab": "b", "order": 1,
                    "scope": {"user": true, "project": false},
                    "detect": {"dirs": []},
                    "format": "identity", "installKind": "perAgent",
                    "dest": {"user": [".b/{slug}"], "project": []}
                }
            }
        }"#;
        let m = parse_manifest(json.as_bytes()).expect("parse");
        assert!(
            m.warnings.iter().any(|w| w.contains("duplicate id")),
            "expected duplicate-id warning, got: {:?}",
            m.warnings
        );
    }

    #[test]
    fn mismatched_key_and_kebab_is_a_schema_error() {
        // The key in the top-level dict is `wrong`, but the
        // entry's `kebab` field is `correct`. Strict mode
        // refuses to guess which one the catalog meant.
        let json = r#"{
            "tools": {
                "wrong": {
                    "id": "x", "label": "X", "short": "X",
                    "kebab": "correct", "order": 0,
                    "scope": {"user": true, "project": false},
                    "detect": {"dirs": []},
                    "format": "identity", "installKind": "perAgent",
                    "dest": {"user": [".x/{slug}"], "project": []}
                }
            }
        }"#;
        let err = parse_manifest(json.as_bytes()).expect_err("should reject");
        match err {
            ManifestError::Schema { msg } => {
                assert!(msg.contains("does not match"));
            }
            other => panic!("expected Schema error, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_fails_at_parse() {
        // No `id` field — serde rejects.
        let json = r#"{
            "tools": {
                "no-id": {
                    "label": "X", "short": "X", "kebab": "no-id",
                    "order": 0, "scope": {"user": true, "project": false},
                    "detect": {"dirs": []}, "format": "identity",
                    "installKind": "perAgent",
                    "dest": {"user": [".x/{slug}"], "project": []}
                }
            }
        }"#;
        assert!(matches!(
            parse_manifest(json.as_bytes()),
            Err(ManifestError::Json { .. })
        ));
    }
}
