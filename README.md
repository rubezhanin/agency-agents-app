# Agency Agents

> A cross-platform, multilingual installer for AI agents. Hermes-aware.
> Maintained by [Yuri Shvets](https://github.com/rubezhanin).

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Built with Tauri 2](https://img.shields.io/badge/Built%20with-Tauri%202-orange)](https://tauri.app)
[![Cross-platform](https://img.shields.io/badge/macOS%20%7C%20Windows%20%7C%20Linux-3A3A3A)](#install)
[![11 locales](https://img.shields.io/badge/i18n-11%20locales-blue)](#features)
[![Hermes-ready](https://img.shields.io/badge/Hermes-plugin%20ready-7C3AED)](#supported-install-targets)
[![Sponsor](https://img.shields.io/badge/♥-Sponsor-EC4899?logo=githubsponsors&logoColor=white)](https://github.com/sponsors/rubezhanin)

Agency Agents is a small, native app for browsing, installing, and tracking the agent personas from [`rubezhanin/agency-agents`](https://github.com/rubezhanin/agency-agents) across the AI coding tools you actually use.

It is full source, MIT-licensed, local-first, and does not run telemetry.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="landing/screenshots/dashboard-dark.png">
  <img alt="Agency Agents — Dashboard: install health, cross-tool coverage, and the catalog by division" src="landing/screenshots/dashboard-light.png">
</picture>

## Why This Exists

The `agency-agents` repo is a useful catalog of specialist AI agent personas, but every coding tool has its own agent format and install path. Claude Code, Codex, Cursor, Gemini CLI, Qwen, opencode, Copilot, and Osaurus all want similar content in slightly different places.

Agency Agents gives that catalog a native control surface:

- browse the agent catalog by division and role
- inspect the source persona before installing it
- install deterministic renders into supported tools
- track what the app wrote using a local ledger
- detect drift when a file was modified outside the app
- update, remove, or back up installs without guessing

The core idea is simple: AI tools do not share a package database, so the app keeps the local install database they are missing.

## Features

Agency Agents is organized around four pillars — **Agents** (who), **Tools** (how), **Teams** (which), and **Projects** (where):

- **Agents workspace** — searchable three-pane catalog, division and category filters, an install-state lens, a detail panel, and per-agent deployment controls.
- **Tools panel** — shows all recognized tools from the registry, detected installs, counts, versions where available, default targets, project installs, and bulk operations. Installable targets render in full; recognized-only targets appear dimmed.
- **Teams** — app-bundled preset teams plus your own saved teams; open a team for a detail panel with Deploy built in. (Teams replaces the earlier "Loadouts" concept; Agentfile export/import remains.)
- **Projects** — project-scoped installs with a dedicated panel and master/detail navigation, so a project gets exactly the agents and tools it needs.
- **Install tracking** — records every app-managed install with source hash, rendered hash, tool, destination, scope, and project path where relevant.
- **Reconciliation** — classifies installed files as current, outdated, modified, removed, or foreign by re-rendering canonical source and comparing bytes. The Dashboard surfaces what "needs attention," and the Agents pane filters to exactly those.
- **Auto-update** — checks a signed update manifest and installs new versions in place, verified against an embedded minisign public key, with one-click install + relaunch. Live for macOS (Apple Silicon + Intel) since v0.2.0; opt-in and gated by Settings.
- **Tool registry** — tool knowledge lives in a single upstream-owned `tools.json` shared by the backend and frontend; adding a tool is editing one JSON entry, and installability is derived from whether the app ships a renderer for that tool's format.
- **Dashboard** — install health, a Global-vs-Projects install sunburst, cross-tool coverage merged with the catalog-by-division view (linked hover), and deep links back into the workspace.
- **GitHub integration** — optional OAuth Device Flow for GitHub-backed app features. Tokens are stored in the platform keychain and are never returned to the frontend.
- **Offline-first catalog** — ships with a bundled corpus baseline and can use a local or managed clone of `agency-agents`.
- **Cross-platform shell** — Tauri 2 + Svelte 5 frontend with native macOS chrome and opaque native windows on Windows/Linux.
- **Multilingual UI** — 11 built-in locales: `en`, `de`, `es`, `fa`, `fr`, `ja`, `ko`, `pt-BR`, `ru`, `zh-CN`, `zh-TW`. Switch in Settings → Appearance; all categories, runbooks, and update banners are localized. New translations are contributed under `src/lib/i18n/locales/`.
- **Hermes-aware** — ships a full plugin-style installer for the `hermes` CLI (see *Supported Install Targets* below).
- **Backups & rollback** — every install/update that overwrites a different file takes a snapshot first into `app_data/backups/`. Settings → Backups lets the user list, tail, and roll any snapshot back to its original `dest` (path-sandboxed to the user home).
- **Structured logs** — every `tracing` event lands in `app_data/logs/app.YYYY-MM-DD.json` (daily rotation) plus stderr. Settings → Logs shows the file list, tails the most recent, and clears the directory. The companion `ts-rs` build keeps Rust DTOs and the TypeScript frontend in lockstep — drift is a CI-failing `git diff`.
- **Accessibility** — skip-to-content link, focus return on modal close, `:focus-visible` global ring, and roving-tabindex keyboard nav in the sidebar.
- **Path-sandboxed IPC** — every user-supplied path (loadout import/export, catalog source picker) is canonicalised and required to resolve inside the user's home before it touches the disk.

New to directing agents? See **[docs/USING-AGENTS.md](./docs/USING-AGENTS.md)** — the Playbook: how to get shipped, tested work out of the catalog (also in-app via the title-bar book icon).

## Supported Install Targets

The app currently installs to the renderer-backed targets that have deterministic byte parity with the upstream `agency-agents` converter, plus a first-class Hermes plugin target:

| Tool | Scope Today | Output | Kind |
|------|-------------|--------|------|
| Claude Code | user | `~/.claude/agents/*.md` | per-agent |
| Codex | user | `~/.codex/agents/*.toml` | per-agent |
| Gemini CLI | user | `~/.gemini/agents/*.md` | per-agent |
| GitHub Copilot | user | `~/.github/agents/*.md` and `~/.copilot/agents/*.md` | per-agent |
| Qwen Code | user | `~/.qwen/agents/*.md` | per-agent |
| Cursor | project | `.cursor/rules/*.mdc` | per-agent |
| opencode | project | `.opencode/agents/*.md` | per-agent |
| Osaurus | user | `~/.osaurus/skills/agency-<slug>/SKILL.md` | per-agent |
| **Hermes** | user | `~/.hermes/plugins/agency-agents-router/` | **plugin** (full) |

**Hermes** is a special case: it ships as a *plugin* (a `agency-agents-router` directory with `manifest.yaml` and skill files), so the app's renderer for it is a directory-producer instead of a single-file renderer. The user installs the plugin either directly (we copy the directory to `~/.hermes/plugins/`) or via the `hermes` CLI (`hermes plugin install …`). See **[docs/HERMES-PLUGIN.md](./docs/HERMES-PLUGIN.md)** for the manifest schema and reconciliation rules.

The upstream AA repo also contains integrations for Antigravity, Aider, Windsurf, OpenClaw, and Kimi. Those output shapes need additional app work before they should be exposed as first-class app installs — they appear in the Tools panel as recognized-only.

## What This Isn't

- Not an agent runtime. The app installs personas into other tools; it does not execute them.
- Not a replacement for the `agency-agents` repo. The repo remains the source catalog.
- Not a telemetry product. There are no analytics SDKs, user tracking, or accounts required for core use.
- Not a shell command bridge. The frontend cannot construct arbitrary shell commands.

## Install

Grab the build for your platform from the [latest release](https://github.com/rubezhanin/agency-agents-app/releases/latest):

- **macOS** (Apple Silicon & Intel) — signed + notarized `.dmg`, macOS 13+.
- **Linux** (x86_64) — `.deb`, `.rpm`, or the portable `.AppImage`.
- **Windows** (x64 & ARM64) — `.exe` installer (not code-signed yet; SmartScreen → *More info → Run anyway*).

Or on macOS via Homebrew:

```sh
brew tap rubezhanin/agency-agents
brew install --cask agency-agents
```

For local review, use the development app:

```sh
npm install
npm run tauri dev
```

For a signed release build on macOS, see [docs/BUILD.md](./docs/BUILD.md).

## Build From Source

Prerequisites:

- [Rust](https://rustup.rs/) stable
- [Node.js 22+](https://nodejs.org/) and npm
- Xcode Command Line Tools on macOS: `xcode-select --install`
- Full Xcode only when regenerating the macOS Liquid Glass icon assets

Then:

```sh
git clone https://github.com/rubezhanin/agency-agents-app
cd agency-agents-app
npm install
npm run tauri dev
npm run check
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
```

The Phase C local QA batch is:

```sh
npm run build:phase-c
```

Use the full VM-assisted batch when the configured Ubuntu/Windows test environments are available:

```sh
npm run build:phase-c:full
```

## Architecture

A Tauri 2 shell hosts a SvelteKit + Svelte 5 frontend in the system WebView. The Rust backend owns the catalog, renderer, install ledger, reconciliation, GitHub integration, settings, and updater boundary.

The catalog comes from `agency-agents`, either as:

- a bundled baseline inside the app
- a managed local clone at `~/.agency-agents`
- a user-selected clone, such as `~/agency-agents` or any path on disk

Rendering is native Rust, deterministic, and tested against the upstream `scripts/convert.sh` outputs for the supported transform tools. The app does not shell out to converter scripts at runtime.

Important implementation areas:

- [src-tauri/src/corpus/mod.rs](./src-tauri/src/corpus/mod.rs) — catalog source, indexing, refresh, category discovery
- [src-tauri/src/render/mod.rs](./src-tauri/src/render/mod.rs) — per-tool deterministic rendering and destination paths
- [src-tauri/src/install/mod.rs](./src-tauri/src/install/mod.rs) — install, uninstall, ledger, detection, reconciliation
- [src/lib/components/AgentsWorkspace.svelte](./src/lib/components/AgentsWorkspace.svelte) — main browse/install surface
- [src/lib/components/ToolsView.svelte](./src/lib/components/ToolsView.svelte) — tool status and bulk operations

Memory Bank design context lives under [memory-bank/](./memory-bank/). Start with [memory-bank/projectbrief.md](./memory-bank/projectbrief.md), [memory-bank/systemPatterns.md](./memory-bank/systemPatterns.md), and [memory-bank/NEXT-SESSION.md](./memory-bank/NEXT-SESSION.md).

## Network Posture

Core browsing and install tracking are local. Network access is explicit and gated by Settings.

Known outbound paths:

- GitHub/codeload/raw GitHub endpoints for refreshing the `agency-agents` catalog when the user requests or enables it.
- GitHub OAuth Device Flow when the user chooses to sign in.
- GitHub API calls for optional GitHub-backed app features.
- The app updater manifest and release artifacts when update checks are enabled.

No telemetry, crash reporting, advertising pixels, or product analytics are included.

## Security

Agency Agents uses typed Tauri IPC commands and avoids `tauri-plugin-shell`. File writes are restricted to known install destinations, app state, backups, and user-selected paths. Modified installed files are backed up before destructive operations.

Report vulnerabilities using [SECURITY.md](./SECURITY.md).

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](./CONTRIBUTING.md).

The highest-value areas before 1.0 are:

- verified tool-target manifest shared with the AA repo
- additional project-scope install targets
- multi-file renderer support for Aider, Windsurf, OpenClaw, Antigravity, and Kimi once their target formats are verified
- Windows/Linux packaging validation
- GitHub issue/discussion integrations

## License

[MIT](./LICENSE). Do whatever you want with this.

## Acknowledgments

- [Agency Agents](https://github.com/rubezhanin/agency-agents) — the source catalog and upstream converter/install scripts. The app contributes its transforms back upstream: v0.2.0's Osaurus integration and the shared `tools.json` tool manifest (the twin of `divisions.json`) landed there first.
- [Tauri](https://tauri.app) — native app shell without the Electron footprint.
- [Svelte](https://svelte.dev) — the frontend runtime.

## Support The Project

If Agency Agents saves you time, consider [sponsoring on GitHub](https://github.com/sponsors/rubezhanin). Sponsorship is optional and does not unlock a paid tier.
