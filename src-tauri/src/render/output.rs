//! Output rendering — `output_slug`, `render`, `render_with_hash`.
//!
//! `render` is the load-bearing deterministic conversion. It's a pure
//! function of `(Agent, raw corpus source, tool id)`, with no timestamps
//! or randomness, so the resulting hash is reproducible across runs.
//! That reproducibility is what `install::reconcile` keys on to decide
//! "is this file ours or did the user edit it?".
//!
//! ## Format dispatch
//!
//! We dispatch on the registry's `format` key (not a Rust variant) so
//! the conversion is data-driven and adding a new tool only touches
//! `tools.json`. The match arm body for each format is the actual
//! emission — kept in one place so the upstream-parity test (see
//! `tests.rs::upstream_convert_sh_is_byte_identical_for_transform_tools`)
//! can compare the whole emit pipeline to `scripts/convert.sh`.

use crate::error::AppError;
use crate::registry;
use crate::types::Agent;

use super::helpers::{
    resolve_opencode_color, slugify, source_body, source_field, toml_escape, unsupported,
};

/// Filename stem emitted by `convert.sh`. Identity tools preserve the source
/// filename; transform tools derive it from frontmatter `name`.
pub fn output_slug(agent: &Agent, raw_source: &str, tool: &str) -> String {
    // Identity tools (`slugFrom: "source"`) keep the corpus filename; transform
    // tools derive the stem from frontmatter `name`, with an optional namespace
    // prefix (skill-dir tools share a global folder, so we prefix "agency-").
    let meta = registry::get(tool);
    if meta.and_then(|m| m.slug_from.as_deref()) == Some("source") {
        agent.slug.clone()
    } else {
        let prefix = meta.and_then(|m| m.slug_prefix.as_deref()).unwrap_or("");
        format!("{prefix}{}", slugify(source_field(raw_source, "name")))
    }
}

/// Render the file content for `tool` from `agent` (+ the raw corpus `.md`
/// source, used verbatim by identity tools). Deterministic.
pub fn render(_agent: &Agent, raw_source: &str, tool: &str) -> Result<String, AppError> {
    let name = source_field(raw_source, "name");
    let description = source_field(raw_source, "description");
    let body = source_body(raw_source);
    let slug = slugify(name);
    // Dispatch on the registry's render `format` key rather than a Rust variant.
    let format = registry::get(tool).and_then(|m| m.format.as_deref());
    let out = match format {
        // Identity — ship the corpus `.md` exactly as authored.
        Some("identity") => raw_source.to_string(),

        // Cursor `.mdc`: description + globs + alwaysApply frontmatter.
        Some("cursor-mdc") => format!(
            "---\ndescription: {desc}\nglobs: \"\"\nalwaysApply: false\n---\n{body}\n",
            desc = description,
        ),

        // Codex TOML: minimal required fields, control chars escaped.
        Some("codex-toml") => format!(
            "name = \"{name}\"\ndescription = \"{desc}\"\ndeveloper_instructions = \"{body}\"\n",
            name = toml_escape(name),
            desc = toml_escape(description),
            body = toml_escape(&body),
        ),

        // Gemini CLI subagent `.md`: name(=slug) + description frontmatter.
        Some("gemini-md") => format!(
            "---\nname: {slug}\ndescription: {desc}\n---\n{body}\n",
            desc = description,
        ),

        // Qwen Code SubAgent `.md`: optional tools line is preserved literally.
        Some("qwen-md") => {
            let tools = source_field(raw_source, "tools");
            if tools.is_empty() {
                format!("---\nname: {slug}\ndescription: {description}\n---\n{body}\n")
            } else {
                format!(
                    "---\nname: {slug}\ndescription: {description}\ntools: {tools}\n---\n{body}\n"
                )
            }
        }

        // ZCode agent `.md` (Z.ai GLM harness): name + description frontmatter,
        // optional `tools` list preserved literally, persona as the body. Read
        // from `.zcode/agents/` (project) or `~/.config/zcode/agents/` (global).
        Some("zcode-md") => {
            let tools = source_field(raw_source, "tools");
            if tools.is_empty() {
                format!("---\nname: {slug}\ndescription: {description}\n---\n{body}\n")
            } else {
                format!(
                    "---\nname: {slug}\ndescription: {description}\ntools: {tools}\n---\n{body}\n"
                )
            }
        }

        // Agent-Skills `SKILL.md`: name (namespaced) + description frontmatter,
        // persona as the body. Mirrors upstream convert.sh `convert_osaurus`
        // (~/.osaurus/skills/<name>/SKILL.md). The `agency-` prefix on `name`
        // comes from the tool's `slugPrefix`.
        Some("skill-md") => {
            let prefix = registry::get(tool).and_then(|m| m.slug_prefix.as_deref()).unwrap_or("");
            format!(
                "---\nname: {prefix}{slug}\ndescription: {desc}\n---\n{body}\n",
                desc = description,
            )
        }

        // OpenCode `.md`: name + description + mode + hex color frontmatter.
        Some("opencode-md") => format!(
            "---\nname: {name}\ndescription: {desc}\nmode: subagent\ncolor: '{color}'\n---\n{body}\n",
            desc = description,
            color = resolve_opencode_color(source_field(raw_source, "color")),
        ),

        // No format (recognized-only) or an unknown renderer ⇒ not installable.
        _ => return Err(unsupported(tool)),
    };
    Ok(out)
}

/// Render + hash in one shot.
pub fn render_with_hash(
    agent: &Agent,
    raw_source: &str,
    tool: &str,
) -> Result<(String, String), AppError> {
    let bytes = render(agent, raw_source, tool)?;
    let hash = super::helpers::sha256_hex(bytes.as_bytes());
    Ok((bytes, hash))
}
