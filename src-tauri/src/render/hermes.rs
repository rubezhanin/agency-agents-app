//! Hermes plugin renderer — produces a multi-file plugin directory.
//!
//! The Hermes plugin format is documented in `docs/HERMES-PLUGIN.md`. Unlike
//! the rest of the supported tools, a Hermes plugin is a **directory**:
//!
//! ```text
//! ~/.hermes/plugins/agency-agents-router/
//! ├── manifest.yaml
//! ├── SKILL.md                       (router skill — main entry point)
//! └── skills/<slug>.md               (one per declared agent)
//! ```
//!
//! The renderer is **deterministic**: the same `(agents, sources, catalog_ref,
//! app_version)` tuple produces the byte-identical directory every time. The
//! install ledger records the plugin as a single install with N child hashes,
//! and reconciliation compares bytes against the recorded catalog_ref.
//!
//! This module is intentionally self-contained. It does NOT participate in
//! `render::render()`'s `match format` dispatch (that function is one-file-only
//! and is the wrong abstraction for directory producers). The installer code
//! in `commands::hermes` calls `render_plugin()` directly and writes the
//! result to `~/.hermes/plugins/agency-agents-router/`.
//!
//! ## Determinism contract
//!
//! - `skills/` is sorted by slug, ascending.
//! - The manifest is hand-rolled YAML with a stable key order.
//! - No timestamps, no random ids, no environment-dependent strings.
//! - `serde_yaml` is intentionally NOT used (its output is non-deterministic
//!   for the same input). A small hand-rolled emitter lives in `yaml_emit`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::render::sha256_hex;
use crate::types::Agent;

/// Default plugin id. Stable across versions. Used as both the install
/// directory name (`~/.hermes/plugins/<PLUGIN_ID>/`) and the
/// `manifest.yaml` `id` field. Custom plugins (multi-plugin routing)
/// pass their own `plugin_id` to `render_named_plugin` /
/// `user_install_root_for`.
pub const PLUGIN_ID: &str = "agency-agents-router";

/// Default plugin label. Mirrored in `manifest.yaml` `display_name` and
/// in the router skill's `# Heading`.
pub const PLUGIN_LABEL: &str = "Agency Agents Router";

/// The user-scoped install root for a given home. Mirrors
/// `tools.json` → `hermes` → `dest.user[0]` (`.hermes/plugins/agency-agents-router`).
#[allow(dead_code)] // public API kept for symmetry with `user_install_root_for`
pub fn user_install_root(home: &Path) -> PathBuf {
    user_install_root_for(home, PLUGIN_ID)
}

/// The user-scoped install root for a custom plugin id. Used by the
/// multi-plugin routing path (Phase 4b) where each division can ship
/// as its own plugin under `~/.hermes/plugins/<id>/`.
pub fn user_install_root_for(home: &Path, plugin_id: &str) -> PathBuf {
    home.join(".hermes").join("plugins").join(plugin_id)
}

/// The on-disk plugin, ready to be written to a destination directory.
///
/// All fields are owned strings; `install_to` writes them to disk.
#[derive(Debug)]
pub struct HermesPlugin {
    /// Stable id used as the install directory basename and the
    /// `manifest.yaml` `id` field. Default: `PLUGIN_ID`. Custom
    /// plugins (Phase 4b multi-plugin routing) get their own id.
    pub plugin_id: String,
    /// Human-readable label mirrored in `manifest.yaml` `display_name`
    /// and the router skill's `# Heading`. Default: `PLUGIN_LABEL`.
    #[allow(dead_code)] // surfaced via `manifest` / `router` bytes; read by the IPC `hermes_list_plugins` scanner.
    pub plugin_label: String,
    /// `manifest.yaml` content (UTF-8 YAML).
    pub manifest: String,
    /// `SKILL.md` content (the router skill).
    pub router: String,
    /// `(slug, skill_body)` pairs, sorted by slug for determinism. One per
    /// declared agent. The skill body is the persona .md as authored.
    pub skills: Vec<(String, String)>,
    /// The app version that produced the plugin (mirrored in
    /// `plugin_meta.version`). Retained on the struct so parity tests and
    /// future logging can compare renderer output to a known app version.
    #[allow(dead_code)]
    pub app_version: String,
    /// The catalog git ref the plugin was rendered against (mirrored in
    /// `plugin_meta.catalog.ref`). Same retention rationale as
    /// `app_version`.
    #[allow(dead_code)]
    pub catalog_ref: String,
}

impl HermesPlugin {
    /// Total number of files the plugin contains: 1 manifest + 1 router + N skills.
    pub fn file_count(&self) -> usize {
        2 + self.skills.len()
    }

    /// SHA-256 of the manifest bytes.
    pub fn manifest_hash(&self) -> String {
        sha256_hex(self.manifest.as_bytes())
    }

    /// SHA-256 of the router SKILL.md bytes.
    pub fn router_hash(&self) -> String {
        sha256_hex(self.router.as_bytes())
    }

    /// `Vec<(slug, sha256)>` for every skill file, in slug order.
    pub fn skill_hashes(&self) -> Vec<(String, String)> {
        self.skills
            .iter()
            .map(|(slug, body)| (slug.clone(), sha256_hex(body.as_bytes())))
            .collect()
    }
}

/// Render the plugin for the canonical `agency-agents-router` plugin.
/// Shortcut for `render_named_plugin(..., PLUGIN_ID, PLUGIN_LABEL)`;
/// kept for backward compatibility with the existing tests + IPC.
pub fn render_plugin(
    agents: &[Agent],
    sources: &[String],
    catalog_ref: &str,
    app_version: &str,
) -> Result<HermesPlugin, AppError> {
    render_named_plugin(
        agents,
        sources,
        catalog_ref,
        app_version,
        PLUGIN_ID,
        PLUGIN_LABEL,
    )
}

/// Render a plugin for an arbitrary plugin id + label. This is the
/// multi-plugin routing path (Phase 4b) where a user can ship one
/// division — say, `engineering` — as its own plugin, distinct from
/// the canonical `agency-agents-router`.
///
/// * `agents`   — the parsed agent list (one `Agent` per persona).
/// * `sources`  — the raw `.md` for each agent, in the same order. The body
///                is shipped verbatim; the renderer does NOT rewrite persona
///                prose. Required frontmatter (`id`, `name`, `description`,
///                `role`, `division`, `version`) is validated up front.
/// * `catalog_ref` — the catalog git sha/tag the agents were read from.
///                    Frozen in the manifest. Reconciliation compares bytes
///                    against this ref.
/// * `app_version` — the app's semver, mirrored in `plugin_meta.version`.
/// * `plugin_id`   — the plugin's stable id (`[a-z0-9-]+`), used as the
///                    install directory basename and the `manifest.yaml`
///                    `id` field.
/// * `plugin_label` — human-readable label for `display_name` and the
///                    router skill's `# Heading`.
///
/// Returns a `HermesPlugin` whose bytes are byte-deterministic given
/// identical inputs.
pub fn render_named_plugin(
    agents: &[Agent],
    sources: &[String],
    catalog_ref: &str,
    app_version: &str,
    plugin_id: &str,
    plugin_label: &str,
) -> Result<HermesPlugin, AppError> {
    if agents.len() != sources.len() {
        return Err(AppError::InvalidArgument {
            message: format!(
                "render_named_plugin: agents ({}) and sources ({}) length mismatch",
                agents.len(),
                sources.len()
            ),
        });
    }
    if agents.is_empty() {
        return Err(AppError::InvalidArgument {
            message: "render_named_plugin: at least one agent is required".into(),
        });
    }
    if catalog_ref.trim().is_empty() {
        return Err(AppError::InvalidArgument {
            message: "render_named_plugin: catalog_ref must be a non-empty git ref".into(),
        });
    }
    if !is_valid_plugin_id(plugin_id) {
        return Err(AppError::InvalidArgument {
            message: format!(
                "render_named_plugin: plugin_id {plugin_id:?} must match [a-z0-9-]+ and be 1-64 chars"
            ),
        });
    }
    if plugin_label.trim().is_empty() {
        return Err(AppError::InvalidArgument {
            message: "render_named_plugin: plugin_label must be non-empty".into(),
        });
    }

    // Validate every persona's required frontmatter before we touch any output.
    for (a, s) in agents.iter().zip(sources.iter()) {
        validate_persona_source(a, s)?;
    }

    // Skills: pair (slug, body) — body is the persona .md exactly as authored.
    let mut skills: Vec<(String, String)> = agents
        .iter()
        .zip(sources.iter())
        .map(|(a, s)| (a.slug.clone(), s.clone()))
        .collect();
    skills.sort_by(|a, b| a.0.cmp(&b.0));

    let manifest = render_manifest(agents, catalog_ref, app_version, plugin_id, plugin_label)?;
    let router = render_router(agents, plugin_id, plugin_label);

    Ok(HermesPlugin {
        plugin_id: plugin_id.to_string(),
        plugin_label: plugin_label.to_string(),
        manifest,
        router,
        skills,
        app_version: app_version.to_string(),
        catalog_ref: catalog_ref.to_string(),
    })
}

/// True when `id` is a safe kebab-case plugin id. Mirrors the
/// hermes-cli naming rule so the install directory basename can
/// double as the manifest's `id` field without quoting.
fn is_valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Write the plugin to a destination directory **atomically**:
/// 1. Stage every file into `<dest_root>.tmp-<uuid>/`.
/// 2. fsync the directory.
/// 3. Rename the staging dir over the existing one (POSIX) or remove + create
///    on Windows (we use `fs::remove_dir_all` + `fs::rename` which the Rust
///    std implements as a fallback for cross-platform safety).
///
/// Returns an `InstallReport` with the per-file SHA-256 hashes for the ledger.
pub fn install_to(plugin: &HermesPlugin, dest_root: &Path) -> Result<InstallReport, AppError> {
    use uuid::Uuid;

    let parent = dest_root
        .parent()
        .ok_or_else(|| AppError::InvalidArgument {
            message: "install_to: dest_root has no parent".into(),
        })?;
    let staging = parent.join(format!(".{}.tmp-{}", plugin.plugin_id, Uuid::new_v4()));

    // 1. Stage the entire directory tree.
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| io_err("staging cleanup", &staging, e))?;
    }
    fs::create_dir_all(&staging).map_err(|e| io_err("create staging", &staging, e))?;
    fs::create_dir_all(staging.join("skills"))
        .map_err(|e| io_err("create skills/", &staging, e))?;

    write_file(&staging.join("manifest.yaml"), plugin.manifest.as_bytes())?;
    write_file(&staging.join("SKILL.md"), plugin.router.as_bytes())?;
    for (slug, body) in &plugin.skills {
        write_file(
            &staging.join("skills").join(format!("{slug}.md")),
            body.as_bytes(),
        )?;
    }

    // 2. Move into place. If something exists, remove it first (idempotent).
    if dest_root.exists() {
        fs::remove_dir_all(dest_root).map_err(|e| io_err("remove previous", dest_root, e))?;
    }
    fs::rename(&staging, dest_root).map_err(|e| io_err("rename into place", &staging, e))?;

    // 3. Best-effort fsync on platforms that support it (POSIX).
    #[cfg(unix)]
    {
        if let Ok(dir) = std::fs::File::open(dest_root) {
            let _ = dir.sync_all();
        }
    }

    Ok(InstallReport {
        manifest_hash: plugin.manifest_hash(),
        router_hash: plugin.router_hash(),
        skill_hashes: plugin.skill_hashes(),
    })
}

/// Result of writing a plugin to disk. The ledger records these hashes
/// keyed on the install record.
pub struct InstallReport {
    pub manifest_hash: String,
    pub router_hash: String,
    pub skill_hashes: Vec<(String, String)>,
}

/// Remove a previously-installed plugin directory. Idempotent — succeeds
/// even if the directory does not exist.
pub fn uninstall_from(dest_root: &Path) -> Result<(), AppError> {
    if !dest_root.exists() {
        return Ok(());
    }
    fs::remove_dir_all(dest_root).map_err(|e| io_err("uninstall", dest_root, e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn io_err(op: &str, path: &Path, e: std::io::Error) -> AppError {
    AppError::Io {
        message: format!("hermes plugin renderer: {op} on {}: {e}", path.display()),
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err("create parent", parent, e))?;
    }
    let mut f = fs::File::create(path).map_err(|e| io_err("create", path, e))?;
    f.write_all(bytes).map_err(|e| io_err("write", path, e))?;
    f.sync_all().map_err(|e| io_err("sync", path, e))?;
    Ok(())
}

/// Validate that a persona source has the required frontmatter fields. The
/// renderer is strict on `id == slug` and `division` membership so the
/// manifest invariants hold without surprise.
fn validate_persona_source(agent: &Agent, source: &str) -> Result<(), AppError> {
    // Required frontmatter keys. We parse the literal "key: value" form like
    // the rest of the renderers — no YAML, no serde_yaml (determinism).
    for field in ["id", "name", "division", "role", "description", "version"] {
        let v = source_field(source, field);
        if v.trim().is_empty() {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "persona {} is missing required frontmatter field {field}",
                    agent.slug
                ),
            });
        }
    }
    if source_field(source, "id") != agent.slug {
        return Err(AppError::InvalidArgument {
            message: format!(
                "persona slug/frontmatter id mismatch: file says {}, manifest says {}",
                source_field(source, "id"),
                agent.slug
            ),
        });
    }
    Ok(())
}

/// Match `scripts/lib.sh#get_field`: return the first literal `field: value`
/// line between exact `---` fences. Same as `render::source_field`, but
/// duplicated here to keep this module self-contained for the plugin path.
fn source_field<'a>(source: &'a str, field: &str) -> &'a str {
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

/// Render the router `SKILL.md`. Lists every included persona with a one-line
/// description and a relative path. The "regenerated by the app" comment is
/// a stable, narrow marker that reconciliation can recognise.
///
/// `plugin_id` and `plugin_label` parameterise the frontmatter `name:` and
/// the `# Heading` so custom plugins (multi-plugin routing, Phase 4b)
/// render distinct router skills.
fn render_router(agents: &[Agent], plugin_id: &str, plugin_label: &str) -> String {
    let mut s = String::with_capacity(2048);
    s.push_str("---\n");
    s.push_str(&format!("name: {plugin_id}\n"));
    s.push_str("description: |\n");
    s.push_str("  Route a user request to the right agent persona from the\n");
    s.push_str("  rubezhanin/agency-agents catalog. Use this skill whenever a\n");
    s.push_str("  user describes a coding, design, ops, or content task and\n");
    s.push_str("  the best-fit persona isn't obvious.\n");
    s.push_str("---\n\n");
    s.push_str(&format!("# {plugin_label}\n\n"));
    s.push_str("You are the **router** for the [Agency Agents](https://github.com/rubezhanin/agency-agents)\n");
    s.push_str("catalog inside Hermes. Your job is to read the user's request, pick the right persona from\n");
    s.push_str("the skills in this plugin, and answer as that persona.\n\n");

    s.push_str("## Routing rules\n\n");
    s.push_str("1. Read the user's task carefully. Identify the *primary* domain (frontend, backend, infra, etc.).\n");
    s.push_str("2. Match the domain to a persona in `skills/`. Prefer the most specific match.\n");
    s.push_str("3. **Adopt the persona's voice** — read the matched `skills/<slug>.md` and answer as if you are that agent.\n");
    s.push_str("4. If no clear match exists, ask one short clarifying question.\n");
    s.push_str("5. Never combine two personas in one answer. Pick one; switch only when the user asks.\n\n");

    s.push_str("## Persona index\n\n");
    s.push_str("<!-- This block is regenerated by the Agency Agents app on every install. Do not hand-edit. -->\n");
    for a in agents {
        // One bullet per persona, sorted by slug (the caller already sorts the
        // skills list, but we re-sort agents here for stability).
        s.push_str(&format!("- `{}` → `skills/{}.md`\n", a.slug, a.slug));
    }
    s.push_str("<!-- end generated block -->\n");

    s
}

fn render_manifest(
    agents: &[Agent],
    catalog_ref: &str,
    app_version: &str,
    plugin_id: &str,
    plugin_label: &str,
) -> Result<String, AppError> {
    let mut s = String::with_capacity(2048);

    // Top-level agent-kit fields.
    s.push_str("schema_version: 1\n");
    s.push_str(&format!("id: {plugin_id}\n"));
    s.push_str(&format!("display_name: {plugin_label}\n"));
    s.push_str("description: |\n");
    s.push_str("  Routes the rubezhanin/agency-agents catalog personas into Hermes\n");
    s.push_str("  skills. Generated by the Agency Agents app — see\n");
    s.push_str("  https://github.com/rubezhanin/agency-agents-app.\n");
    s.push_str("privacy: open\n\n");

    // plugin_meta block (Agency-Agents extension).
    s.push_str("plugin_meta:\n");
    s.push_str("  schema_version: 1\n");
    s.push_str(&format!("  name: {plugin_id}\n"));
    s.push_str(&format!("  version: {app_version}\n"));
    s.push_str("  author: Yuri Shvets\n");
    s.push_str("  homepage: https://github.com/rubezhanin/agency-agents-app\n");
    s.push_str("  license: MIT\n");
    s.push_str("  type: router\n");
    s.push_str("  entry: SKILL.md\n");
    s.push_str("  catalog:\n");
    s.push_str("    source: github:rubezhanin/agency-agents\n");
    s.push_str(&format!("    ref: {}\n", yaml_scalar(catalog_ref)));
    s.push_str(&format!("    agents: {}\n", agents.len()));
    s.push('\n');

    // agents: sorted by id for stability.
    let mut sorted_agents: Vec<&Agent> = agents.iter().collect();
    sorted_agents.sort_by(|a, b| a.slug.cmp(&b.slug));

    s.push_str("agents:\n");
    for a in &sorted_agents {
        s.push_str(&format!("  - id: {}\n", yaml_scalar(&a.slug)));
        s.push_str(&format!("    display_name: {}\n", yaml_scalar(&a.name)));
        s.push_str(&format!("    role: {}\n", yaml_scalar(&a.description)));
        s.push_str(&format!("    workspace: {}\n", yaml_scalar(&a.category)));
        s.push_str("    memory_scope: []\n");
        s.push_str(&format!("    skills: [{}]\n", yaml_scalar(&a.slug)));
    }
    s.push('\n');

    // relationships: one edge from a synthetic "router" persona to every
    // included agent. The router itself is *not* an agent in the catalog; it
    // is the plugin's entry skill. Edges are sorted for determinism.
    s.push_str("relationships:\n");
    s.push_str("  edges:\n");
    for a in &sorted_agents {
        s.push_str(&format!(
            "    - {{ from: agency-agents-router, to: {}, kind: routes-to }}\n",
            yaml_scalar(&a.slug)
        ));
    }
    s.push('\n');

    s.push_str("shared_resources: []\n");
    s.push_str("install_modes:\n");
    s.push_str("  routing: kanban\n");
    s.push_str("  auto_install_hermes: false\n");

    Ok(s)
}

/// Quote a YAML scalar if it contains characters that would otherwise need
/// quoting. Keeps simple slugs bare and wraps everything else in double quotes
/// with embedded backslashes and double quotes escaped. The point of doing
/// this by hand is *determinism* — `serde_yaml` would produce a different
/// quoting style for the same input.
fn yaml_scalar(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".into();
    }
    let needs_quote = value.chars().any(|c| match c {
        ':' | '#' | '{' | '}' | '[' | ']' | ',' | '&' | '*' | '!' | '|' | '>' | '\'' | '"'
        | '%' | '@' | '`' | '\n' | '\t' | ' ' => true,
        c if c.is_ascii_control() => true,
        _ => false,
    });
    if !needs_quote
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_agent(slug: &str) -> Agent {
        Agent {
            slug: slug.into(),
            name: format!("{slug} title"),
            description: format!("{slug} description"),
            category: "engineering".into(),
            emoji: None,
            color: None,
            vibe: None,
            body: String::new(),
        }
    }

    fn fixture_source(slug: &str) -> String {
        format!(
            "---\nid: {slug}\nname: {slug} title\ndivision: engineering\nrole: {slug} role\ndescription: {slug} description\nversion: 1.0.0\n---\n\n# {slug}\n\nbody for {slug}.\n"
        )
    }

    #[test]
    fn render_plugin_is_deterministic() {
        let agents = vec![
            fixture_agent("zeta"),
            fixture_agent("alpha"),
            fixture_agent("mu"),
        ];
        let sources = vec![
            fixture_source("zeta"),
            fixture_source("alpha"),
            fixture_source("mu"),
        ];
        let a = render_plugin(&agents, &sources, "abc123", "0.4.0").unwrap();
        let b = render_plugin(&agents, &sources, "abc123", "0.4.0").unwrap();
        assert_eq!(a.manifest, b.manifest);
        assert_eq!(a.router, b.router);
        assert_eq!(a.skills, b.skills);
        assert_eq!(a.manifest_hash(), b.manifest_hash());
    }

    #[test]
    fn render_plugin_sorts_skills_by_slug() {
        let agents = vec![fixture_agent("zeta"), fixture_agent("alpha")];
        let sources = vec![fixture_source("zeta"), fixture_source("alpha")];
        let p = render_plugin(&agents, &sources, "abc", "0.4.0").unwrap();
        assert_eq!(p.skills[0].0, "alpha");
        assert_eq!(p.skills[1].0, "zeta");
    }

    #[test]
    fn render_plugin_rejects_length_mismatch() {
        let agents = vec![fixture_agent("a")];
        let sources = vec![fixture_source("a"), fixture_source("b")];
        let r = render_plugin(&agents, &sources, "abc", "0.4.0");
        assert!(r.is_err());
    }

    #[test]
    fn render_plugin_rejects_empty_agents() {
        let r = render_plugin(&[], &[], "abc", "0.4.0");
        assert!(r.is_err());
    }

    #[test]
    fn render_plugin_rejects_empty_catalog_ref() {
        let agents = vec![fixture_agent("a")];
        let sources = vec![fixture_source("a")];
        let r = render_plugin(&agents, &sources, "  ", "0.4.0");
        assert!(r.is_err());
    }

    #[test]
    fn render_plugin_rejects_id_mismatch() {
        let agents = vec![fixture_agent("a")];
        let mut source = fixture_source("a");
        // corrupt the frontmatter id
        source = source.replace("id: a", "id: b");
        let r = render_plugin(&agents, &[source], "abc", "0.4.0");
        assert!(r.is_err());
    }

    #[test]
    fn install_to_writes_expected_layout() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("plugin");
        let agents = vec![fixture_agent("frontend-architect")];
        let sources = vec![fixture_source("frontend-architect")];
        let plugin = render_plugin(&agents, &sources, "abc", "0.4.0").unwrap();
        let report = install_to(&plugin, &dest).unwrap();

        assert!(dest.join("manifest.yaml").is_file());
        assert!(dest.join("SKILL.md").is_file());
        assert!(dest.join("skills/frontend-architect.md").is_file());
        assert_eq!(report.manifest_hash, plugin.manifest_hash());
        assert_eq!(report.router_hash, plugin.router_hash());
        assert_eq!(report.skill_hashes.len(), 1);
    }

    #[test]
    fn install_to_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("plugin");
        let agents = vec![fixture_agent("a"), fixture_agent("b")];
        let sources = vec![fixture_source("a"), fixture_source("b")];
        let plugin = render_plugin(&agents, &sources, "abc", "0.4.0").unwrap();
        install_to(&plugin, &dest).unwrap();
        install_to(&plugin, &dest).unwrap();
        assert!(dest.join("manifest.yaml").is_file());
        assert!(dest.join("skills/a.md").is_file());
        assert!(dest.join("skills/b.md").is_file());
    }

    #[test]
    fn uninstall_from_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("plugin");
        // Never installed — must succeed.
        uninstall_from(&dest).unwrap();
        uninstall_from(&dest).unwrap();
    }

    #[test]
    fn yaml_scalar_quotes_when_needed() {
        assert_eq!(yaml_scalar("plain"), "plain");
        assert_eq!(yaml_scalar("needs: colon"), "\"needs: colon\"");
        assert_eq!(yaml_scalar("with \"quote\""), "\"with \\\"quote\\\"\"");
        assert_eq!(yaml_scalar(""), "\"\"");
    }

    #[test]
    fn manifest_contains_router_relationships() {
        let agents = vec![fixture_agent("a"), fixture_agent("b")];
        let sources = vec![fixture_source("a"), fixture_source("b")];
        let p = render_plugin(&agents, &sources, "abc", "0.4.0").unwrap();
        assert!(p.manifest.contains("relationships:"));
        assert!(p.manifest.contains("kind: routes-to"));
        // Edges sorted by `to` slug.
        let a_pos = p.manifest.find("to: a").unwrap();
        let b_pos = p.manifest.find("to: b").unwrap();
        assert!(a_pos < b_pos);
    }

    // -----------------------------------------------------------------
    // Phase 4b — multi-plugin routing: custom id / label
    // -----------------------------------------------------------------

    #[test]
    fn is_valid_plugin_id_accepts_kebab() {
        assert!(is_valid_plugin_id("agency-agents-router"));
        assert!(is_valid_plugin_id("a"));
        assert!(is_valid_plugin_id("engineering-team"));
        assert!(is_valid_plugin_id("plugin123"));
    }

    #[test]
    fn is_valid_plugin_id_rejects_unsafe() {
        // Empty, uppercase, dots, slashes, too long.
        assert!(!is_valid_plugin_id(""));
        assert!(!is_valid_plugin_id("MixedCase"));
        assert!(!is_valid_plugin_id("with.dot"));
        assert!(!is_valid_plugin_id("with/slash"));
        assert!(!is_valid_plugin_id("with space"));
        let long: String = "a".repeat(65);
        assert!(!is_valid_plugin_id(&long));
    }

    #[test]
    fn render_named_plugin_uses_custom_id_and_label() {
        let agents = vec![fixture_agent("alpha")];
        let sources = vec![fixture_source("alpha")];
        let plugin = render_named_plugin(
            &agents,
            &sources,
            "abc",
            "1.0.0",
            "engineering-team",
            "Engineering Team Plugin",
        )
        .unwrap();
        assert_eq!(plugin.plugin_id, "engineering-team");
        assert_eq!(plugin.plugin_label, "Engineering Team Plugin");
        // Manifest `id:` field reflects the custom plugin id.
        assert!(plugin.manifest.contains("id: engineering-team\n"));
        assert!(plugin.manifest.contains("display_name: Engineering Team Plugin\n"));
        // Router skill's frontmatter `name:` and `# Heading` reflect the id/label.
        assert!(plugin.router.contains("name: engineering-team\n"));
        assert!(plugin.router.contains("# Engineering Team Plugin\n"));
    }

    #[test]
    fn render_named_plugin_rejects_bad_id() {
        let agents = vec![fixture_agent("alpha")];
        let sources = vec![fixture_source("alpha")];
        let err = render_named_plugin(
            &agents,
            &sources,
            "abc",
            "1.0.0",
            "Mixed Case",
            "Bad",
        )
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("plugin_id"));
    }

    #[test]
    fn render_named_plugin_rejects_empty_label() {
        let agents = vec![fixture_agent("alpha")];
        let sources = vec![fixture_source("alpha")];
        let err = render_named_plugin(
            &agents,
            &sources,
            "abc",
            "1.0.0",
            "ok-id",
            "   ",
        )
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("plugin_label"));
    }

    #[test]
    fn install_to_writes_to_custom_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("custom-plugin");
        let agents = vec![fixture_agent("alpha")];
        let sources = vec![fixture_source("alpha")];
        let plugin = render_named_plugin(
            &agents,
            &sources,
            "abc",
            "1.0.0",
            "engineering-team",
            "Engineering Team",
        )
        .unwrap();
        let report = install_to(&plugin, &dest).unwrap();
        assert!(dest.join("manifest.yaml").is_file());
        assert_eq!(report.skill_hashes.len(), 1);
        // manifest_id matches the custom plugin id.
        let manifest_text = std::fs::read_to_string(dest.join("manifest.yaml")).unwrap();
        assert!(manifest_text.contains("id: engineering-team\n"));
    }
}
