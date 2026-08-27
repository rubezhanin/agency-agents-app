//! Integration tests for the render submodule tree.
//!
//! Covers the full conversion surface (every supported tool),
//! deterministic round-trip, source-helper byte parity with
//! scripts/lib.sh, and the destination-path resolution.

use crate::render::dests::dests;
use crate::render::dispatch::{scope_for, supports_project, supports_user};
use crate::render::helpers::{slugify, source_body, source_field};
use crate::render::output::{output_slug, render};
use crate::types::{Agent, Scope};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::process::Command;

    fn agent() -> Agent {
        Agent {
            slug: "frontend-developer".into(),
            name: "Frontend Developer".into(),
            description: "Builds UIs.".into(),
            category: "engineering".into(),
            emoji: Some("🎨".into()),
            color: Some("blue".into()),
            vibe: Some("Ships pixels.".into()),
            body: "You are a frontend dev.\n".into(),
        }
    }

    fn raw() -> &'static str {
        "---\nname: Frontend Developer\ndescription: Builds UIs.\ncolor: blue\nemoji: 🎨\nvibe: Ships pixels.\n---\nYou are a frontend dev.\n"
    }

    #[test]
    fn claude_code_is_identity() {
        let a = agent();
        let raw = "---\nname: Frontend Developer\n---\nORIGINAL BODY\n";
        assert_eq!(render(&a, raw, "claudeCode").unwrap(), raw);
        assert_eq!(render(&a, raw, "copilot").unwrap(), raw);
    }

    #[test]
    fn cursor_mdc_shape() {
        let out = render(&agent(), raw(), "cursor").unwrap();
        assert!(out
            .starts_with("---\ndescription: Builds UIs.\nglobs: \"\"\nalwaysApply: false\n---\n"));
        assert!(out.contains("You are a frontend dev."));
    }

    #[test]
    fn codex_toml_escapes() {
        let mut a = agent();
        a.description = "has \"quotes\" and\nnewline".into();
        let source = "---\nname: Frontend Developer\ndescription: has \"quotes\" and\tcontrols\n---\nline 1\nline \"2\"\n";
        let out = render(&a, source, "codex").unwrap();
        assert!(out.contains("description = \"has \\\"quotes\\\" and\\tcontrols\""));
        assert!(out.contains("developer_instructions = \"line 1\\nline \\\"2\\\"\""));
        assert!(out.starts_with("name = \"Frontend Developer\""));
    }

    #[test]
    fn opencode_color_maps_to_hex() {
        let out = render(&agent(), raw(), "opencode").unwrap();
        assert!(out.contains("color: '#3498DB'"), "blue → #3498DB: {out}");
        assert!(out.contains("mode: subagent"));
    }

    #[test]
    fn osaurus_skill_md_shape_and_dest() {
        // Mirrors upstream convert.sh `convert_osaurus`: a SKILL.md whose `name`
        // carries the `agency-` namespace prefix, persona as the body.
        let out = render(&agent(), raw(), "osaurus").unwrap();
        assert_eq!(
            out,
            "---\nname: agency-frontend-developer\ndescription: Builds UIs.\n---\nYou are a frontend dev.\n"
        );
        // output_slug carries the prefix → it names the skill directory.
        assert_eq!(
            output_slug(&agent(), raw(), "osaurus"),
            "agency-frontend-developer"
        );
        // dest is the nested ~/.osaurus/skills/<name>/SKILL.md (user-scope).
        let d = dests(
            "osaurus",
            "agency-frontend-developer",
            Path::new("/home"),
            None,
        )
        .unwrap();
        assert_eq!(
            d,
            vec![PathBuf::from(
                "/home/.osaurus/skills/agency-frontend-developer/SKILL.md"
            )]
        );
    }

    #[test]
    fn antigravity_skill_md_shape_and_dests() {
        // Antigravity reuses the skill-md renderer (identical shape to osaurus,
        // same `agency-` prefix). Global skills load from ~/.gemini/config/skills/,
        // project skills from <project>/.agents/skills/.
        let out = render(&agent(), raw(), "antigravity").unwrap();
        assert_eq!(
            out,
            "---\nname: agency-frontend-developer\ndescription: Builds UIs.\n---\nYou are a frontend dev.\n"
        );
        assert_eq!(
            output_slug(&agent(), raw(), "antigravity"),
            "agency-frontend-developer"
        );
        // user-scope → ~/.gemini/config/skills/<name>/SKILL.md
        let user = dests(
            "antigravity",
            "agency-frontend-developer",
            Path::new("/home"),
            None,
        )
        .unwrap();
        assert_eq!(
            user,
            vec![PathBuf::from(
                "/home/.gemini/config/skills/agency-frontend-developer/SKILL.md"
            )]
        );
        // project-scope → <project>/.agents/skills/<name>/SKILL.md
        let proj = dests(
            "antigravity",
            "agency-frontend-developer",
            Path::new("/home"),
            Some(Path::new("/proj")),
        )
        .unwrap();
        assert_eq!(
            proj,
            vec![PathBuf::from(
                "/proj/.agents/skills/agency-frontend-developer/SKILL.md"
            )]
        );
    }

    #[test]
    fn opencode_unknown_color_falls_back() {
        let mut a = agent();
        a.color = None;
        let source = "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBody\n";
        let out = render(&a, source, "opencode").unwrap();
        assert!(out.contains("color: '#6B7280'"));
    }

    #[test]
    fn gemini_uses_slug_as_name() {
        let out = render(&agent(), raw(), "geminiCli").unwrap();
        assert!(out.starts_with("---\nname: frontend-developer\ndescription: Builds UIs.\n---\n"));
    }

    #[test]
    fn render_is_deterministic() {
        for tool in ["cursor", "codex", "opencode", "geminiCli", "qwen", "zcode"] {
            let a = render(&agent(), raw(), tool).unwrap();
            let b = render(&agent(), raw(), tool).unwrap();
            assert_eq!(a, b, "{tool} must be deterministic");
        }
    }

    #[test]
    fn source_helpers_match_shell_semantics() {
        let source = "---\nname: \"Quoted Name\"\ndescription: has: colon\ntools: Read, Write\n---\nBody\n---\nTail\n\n";
        assert_eq!(source_field(source, "name"), "\"Quoted Name\"");
        assert_eq!(source_field(source, "description"), "has: colon");
        assert_eq!(source_body(source), "Body\nTail");
        assert_eq!(slugify("FP&A / QA"), "fp-a-qa");
    }

    #[test]
    fn qwen_preserves_optional_tools() {
        let source = "---\nname: Frontend Developer\ndescription: Builds UIs.\ntools: Read, Write\n---\nBody\n";
        let out = render(&agent(), source, "qwen").unwrap();
        assert!(out.contains("\ntools: Read, Write\n"));

        let without = render(&agent(), raw(), "qwen").unwrap();
        assert!(!without.contains("\ntools: "));
    }

    #[test]
    fn zcode_uses_slug_name_and_optional_tools() {
        // ZCode agent .md: name(=slug) + description frontmatter, optional tools.
        let out = render(&agent(), raw(), "zcode").unwrap();
        assert!(out.starts_with("---\nname: frontend-developer\ndescription: Builds UIs.\n---\n"));
        assert!(!out.contains("\ntools: "));

        let source = "---\nname: Frontend Developer\ndescription: Builds UIs.\ntools: Read, Write\n---\nBody\n";
        let with = render(&agent(), source, "zcode").unwrap();
        assert!(with.contains("\ntools: Read, Write\n"));
    }

    #[test]
    fn output_slug_matches_converter_identity_rules() {
        let mut a = agent();
        a.slug = "engineering-frontend-developer".into();
        assert_eq!(
            output_slug(&a, raw(), "claudeCode"),
            "engineering-frontend-developer"
        );
        assert_eq!(output_slug(&a, raw(), "codex"), "frontend-developer");
    }

    fn collect_markdown(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_markdown(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }

    #[test]
    #[ignore = "requires AGENCY_AGENTS_PARITY_ROOT and executes upstream convert.sh"]
    fn upstream_convert_sh_is_byte_identical_for_transform_tools() {
        let root = std::env::var("AGENCY_AGENTS_PARITY_ROOT")
            .expect("set AGENCY_AGENTS_PARITY_ROOT to an agency-agents clone");
        let root = PathBuf::from(root);
        let script = root.join("scripts/convert.sh");
        assert!(script.is_file(), "missing {}", script.display());

        let script_text = fs::read_to_string(&script).unwrap();
        let dirs_start = script_text.find("AGENT_DIRS=(").expect("AGENT_DIRS");
        let dirs_tail = &script_text[dirs_start + "AGENT_DIRS=(".len()..];
        let dirs_body = dirs_tail.split(')').next().expect("AGENT_DIRS close");
        let categories: Vec<&str> = dirs_body.split_whitespace().collect();

        let temp = tempfile::tempdir().unwrap();
        let tools = [
            ("cursor", "cursor/rules", "mdc"),
            ("codex", "codex/agents", "toml"),
            ("geminiCli", "gemini-cli/agents", "md"),
            ("opencode", "opencode/agents", "md"),
            ("qwen", "qwen/agents", "md"),
            ("zcode", "zcode/agents", "md"),
        ];
        for (_, tool_id, _) in tools {
            let tool = tool_id.split('/').next().unwrap();
            let status = Command::new("bash")
                .arg(&script)
                .args(["--tool", tool, "--out"])
                .arg(temp.path())
                .status()
                .unwrap();
            assert!(status.success(), "convert.sh failed for {tool}");
        }

        let mut files = Vec::new();
        for category in categories {
            collect_markdown(&root.join(category), &mut files);
        }
        files.sort();

        let mut conversion_slugs = HashSet::new();
        let mut compared = 0usize;
        for path in files {
            let raw = fs::read_to_string(&path).unwrap();
            let name = source_field(&raw, "name");
            if name.is_empty() || !raw.starts_with("---\n") {
                continue;
            }
            let source_slug = path.file_stem().unwrap().to_string_lossy().to_string();
            let agent = Agent {
                slug: source_slug,
                name: name.to_string(),
                description: String::new(),
                category: String::new(),
                emoji: None,
                color: None,
                vibe: None,
                body: String::new(),
            };
            let converted_slug = output_slug(&agent, &raw, "codex");
            assert!(
                conversion_slugs.insert(converted_slug.clone()),
                "duplicate conversion slug: {converted_slug}"
            );
            for (tool, subdir, ext) in tools {
                let expected_path = temp
                    .path()
                    .join(subdir)
                    .join(format!("{converted_slug}.{ext}"));
                let expected = fs::read(&expected_path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", expected_path.display()));
                let actual = render(&agent, &raw, tool).unwrap();
                assert_eq!(
                    actual.as_bytes(),
                    expected,
                    "{tool} parity mismatch for {}",
                    path.display()
                );
                compared += 1;
            }
        }
        assert!(compared > 0);
        eprintln!(
            "renderer parity: {} agents, {} byte comparisons",
            conversion_slugs.len(),
            compared
        );
    }

    #[test]
    fn unsupported_tools_error() {
        // These tools are in the catalog (upstream truth: real format + dest), but
        // this app ships no renderer for their format — so render() must refuse.
        // dests() legitimately returns the upstream templates; the install path is
        // gated on render(), and these tools aren't in the installable set anyway.
        for tool in ["windsurf", "aider", "openclaw", "kimi"] {
            assert!(
                render(&agent(), "raw", tool).is_err(),
                "{tool} has no app renderer"
            );
        }
    }

    #[test]
    fn dests_per_tool() {
        let home = Path::new("/Users/x");
        let proj = Path::new("/proj");
        assert_eq!(
            dests("claudeCode", "a", home, None).unwrap(),
            vec![PathBuf::from("/Users/x/.claude/agents/a.md")]
        );
        assert_eq!(dests("copilot", "a", home, None).unwrap().len(), 2);
        assert_eq!(
            dests("codex", "a", home, None).unwrap(),
            vec![PathBuf::from("/Users/x/.codex/agents/a.toml")]
        );
        assert_eq!(
            dests("cursor", "a", home, Some(proj)).unwrap(),
            vec![PathBuf::from("/proj/.cursor/rules/a.mdc")]
        );
        // project-scoped without a project path → error
        assert!(dests("cursor", "a", home, None).is_err());
    }

    #[test]
    fn scope_capabilities() {
        // Dual-scope tools support both global and project; Cursor is project-only.
        assert!(supports_user("claudeCode") && supports_project("claudeCode"));
        assert!(supports_user("opencode") && supports_project("opencode"));
        assert!(supports_user("codex") && supports_project("codex"));
        assert!(!supports_user("cursor") && supports_project("cursor"));
        // An install's scope comes from whether a project root was chosen.
        assert_eq!(scope_for(None), Scope::User);
        assert_eq!(scope_for(Some(Path::new("/p"))), Scope::Project);
    }

    #[test]
    fn dests_are_scope_aware() {
        let home = Path::new("/home/u");
        let proj = Path::new("/work/app");
        // Root-swap tools: same relative path, rooted at home or the project.
        assert_eq!(
            dests("claudeCode", "x", home, None).unwrap()[0],
            home.join(".claude/agents/x.md")
        );
        assert_eq!(
            dests("claudeCode", "x", home, Some(proj)).unwrap()[0],
            proj.join(".claude/agents/x.md")
        );
        // opencode uses DIFFERENT dirs per scope.
        assert_eq!(
            dests("opencode", "x", home, None).unwrap()[0],
            home.join(".config/opencode/agents/x.md")
        );
        assert_eq!(
            dests("opencode", "x", home, Some(proj)).unwrap()[0],
            proj.join(".opencode/agents/x.md")
        );
        // Cursor is project-only: a global (no project root) request errors.
        assert!(dests("cursor", "x", home, None).is_err());
        assert_eq!(
            dests("cursor", "x", home, Some(proj)).unwrap()[0],
            proj.join(".cursor/rules/x.mdc")
        );
    }
}
