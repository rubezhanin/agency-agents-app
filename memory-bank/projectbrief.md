# Project Brief — Agency Agents

**Product**: Agency Agents
**Bundle ID**: `app.rubezhanin.agency-agents-app`
**Repo**: github:rubezhanin/agency-agents-app
**Author**: rubezhanin / Rubezhanin
**License**: MIT · No telemetry · No accounts

## One-liner

A native macOS app store for AI agents. Browse, search, install, and **track** the
**210** specialized agent personas (across 16 categories) from
[`rubezhanin/agency-agents`](https://github.com/rubezhanin/agency-agents)
across every AI coding tool you use (Claude Code, Cursor, Codex, Gemini CLI, Copilot,
Windsurf, Aider, opencode, qwen, openclaw, antigravity).
(The repo also ships NEXUS playbooks + workflow examples — docs, not agents — a candidate
for a future "Playbooks" section.)

## Thesis

This app is to the **agency-agents** repo what **brew-browser** is to **Homebrew**:
a thin, respectful native GUI over a CLI/content catalog. We forked brew-browser
structurally (Tauri 2 + Svelte 5 + Rust) and re-pointed the domain layer from
`brew` packages to AI agents.

**Key difference from brew-browser**: Homebrew owns its own install database
(`brew list`/`brew outdated` are the source of truth). The AI tools have **no such
registry** — every tool's config dir is a dumb dumping ground. So *we must BE the
database*. That state-tracking layer is the app's core differentiator:
> "the only tool that tracks your agents across every AI tool you use."

## What it is / isn't

- **Is**: a browser + installer + cross-tool state tracker + loadout manager for agents.
- **Isn't**: a replacement for the agency-agents repo, telemetry-funded, freemium,
  or a runtime that executes agents itself (v1 installs into other tools; it does not run agents).

## Source of truth (the corpus-copy model)

The app maintains its **own local working copy** of the agency-agents repo in
`~/Library/Application Support/app.rubezhanin.agency-agents-app/corpus/`, seeded from a
bundled baseline and refreshed from the GitHub tarball. From that copy it derives a
hash index (`corpus-index.json`) and renders/install agents into each tool. See
`systemPatterns.md`.

## Inherited wholesale from brew-browser (do not redesign)

Tauri 2 shell · Svelte 5 runes frontend · streaming job model
(`run_*_streaming` → `Channel<Event>` + global write-lock) · design system
(`src/lib/styles/tokens.css`, components) · command palette · settings shell ·
window chrome (overlay titlebar + vibrancy) · minisign in-app updater ·
`sign-and-notarize.sh` / `publish-manifest.sh` · AGENTS.md methodology + memory-bank.

## Status

Phase 0 (fork + rebrand) — DONE. See `phases/phase-roadmap.md` and `activeContext.md`.
