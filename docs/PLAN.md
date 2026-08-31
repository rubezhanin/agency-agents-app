# Agency Agents App Plan

**Product:** Agency Agents  
**Repo:** `github:rubezhanin/agency-agents-app`  
**Catalog:** `github:rubezhanin/agency-agents`  
**Stack:** Tauri 2, Rust, SvelteKit, Svelte 5, TypeScript  
**License:** MIT

## v1.2 Changelog (2026-08-31)

Cut as `v1.2.0` (`80f79a5`). Closes out the **Trustworthy Core** roadmap —
transactional engine, manifest, plan/dry-run, Hermes deep integration, durable
audit log, and team-mode export/clear. All 6 phases plus a Phase 5 follow-up
shipped as 12 commits ahead of `v1.0.0` (plus a v1.1.0 cut on `b96bcff`):

- **Plan / Dry Run** (`48ef528`) — `deploy_plan` IPC renders the install into a
  structured `DeployPlan { changes, summary }` without writing anything.
  Path-sandbox refuses any dest that escapes the user home.
- **Hermes pre-flight** (`72dd020`) — `hermes_preflight` runs 5 independent
  checks (CLI / kanban / Node / home writable / install target) and returns
  a colour-coded checklist.
- **Multi-plugin routing** (`d95a289`) — `render_named_plugin(agents, sources,
  catalog_ref, app_version, plugin_id, plugin_label)` parameterises the
  renderer so each division can ship as its own plugin.
- **Aggregated health + 60s auto-refresh** (`eabcccc`) — `hermes_health`
  bundles probe + preflight + installed-plugins into a single atomic snapshot
  with `HermesHealthStatus = ok | degraded | down`. The Settings tile polls
  on a 60s `setInterval` (lifecycle via onMount/onDestroy).
- **DeployPreview modal** (`2a57535`) — UI for the Phase 3 plan: colour-coded
  checklist of creates/overwrites/unchanged/refused, destructive-warning
  callout, "Proceed anyway" wording for destructive plans.
- **Durable audit log** (`4b57815`) — `app_data/audit/operations.jsonl`,
  append-only, `create + append + sync_all`, ts-rs `AuditEntry` DTO. New
  Settings → Audit section; install store emits one entry per success/fail.
- **Team-mode export / clear** (`ebc0b15`) — `audit_export(dest)` writes a
  pretty-printed JSON array atomically through `.tmp-<uuid>` + `rename`;
  `audit_clear()` truncates the log behind a `window.confirm` gate.
- **Runbook apply** (`c110749`, v1.3.0 pending) — `runbook_apply(slug, tool)`
  IPC resolves the NEXUS runbook to a deduplicated slug list and returns a
  structured `RunbookApplySummary`; the UI gets a dedicated "Apply all" button
  that drives the per-slug installs through the existing `install` store.
- **v1.1.0 cut** (`b96bcff`) — version bump for the Phase 3+4 + DeployPreview
  release.

## Vision

Ship a native app for browsing, installing, and tracking the `agency-agents` catalog across AI coding tools.

The app should answer three questions clearly:

1. Which agents exist?
2. Where are they installed?
3. Are those installed files current, modified, missing, or foreign?

## Current Architecture

```text
Svelte UI                      four pillars: Agents / Tools / Teams / Projects
  Agents workspace
  Tools panel
  Teams (preset + saved)
  Projects (project-scoped installs)
  Dashboard
  Playbook
  Settings
      |
      | typed Tauri IPC
      v
Rust backend
  corpus/     catalog source, refresh, indexing
  registry    single-source tools.json (shared with the frontend)
  render/     deterministic tool renderers (format-dispatched)
  install/    write, uninstall, backups, ledger, reconcile
  github/     optional OAuth + GitHub API features
  settings/   local settings and network gates
  updater/    manifest fetch + minisign verify (present, endpoint not yet live)
      |
      v
Local filesystem
  app state
  agency-agents clone/baseline
  tool-specific agent directories
```

## MVP Scope

In scope:

1. Browse the `agency-agents` catalog by division, search, and detail.
2. Select a bundled, managed, or user-cloned catalog source.
3. Render supported tools natively in Rust.
4. Install and uninstall supported one-file-per-agent targets.
5. Track local install state with a ledger.
6. Reconcile disk state into current, outdated, modified, removed, and foreign.
7. Back up divergent files before removal or overwrite.
8. Show tool coverage and project targets.
9. Build signed macOS artifacts and cross-platform development builds.

Out of scope for the current release:

- executing agents
- arbitrary third-party plugin execution
- telemetry
- cloud sync
- paid tiers
- unverified install paths
- multi-file/aggregate renderers unless explicitly implemented and tested

## Supported Renderer Set

Current app-supported (installable) targets — 8:

- Claude Code
- Codex
- Gemini CLI
- GitHub Copilot
- Qwen Code
- Cursor
- opencode
- Osaurus (`skill-md` → `~/.osaurus/skills/agency-<slug>/SKILL.md`)

Known AA repo targets that still need app support (recognized-only in the Tools panel) — 5:

- Antigravity — blocked on upstream: `convert_antigravity()` still stamps a non-deterministic `date_added: '${TODAY}'`, so byte-parity is impossible until that field is removed or made deterministic.
- Aider
- Windsurf
- OpenClaw
- Kimi

## Near-Term Plan

### Phase A: Core Workspace

Done. Unified Agents/Library workspace with deployment matrix, search, filters, and persistent detail panel.

### Phase B: Dashboard And Tools Console

Done. Coverage charts, health summaries, category distribution, tool list/detail console, and deep links.

### Phase C: Cross-Platform Correctness

Mostly done. macOS retains overlay titlebar and vibrancy. Windows/Linux use opaque native decorated windows. Remaining work is repeatable build automation and native runtime verification on available VMs.

### Phase D: Tool Target Manifest

Done (v0.2.0). Shipped as the upstream-owned single `tools.json` — the twin of `divisions.json` —
declaring id, label, scopes, detect paths, version probe, output format, and destinations per tool.
Both the Rust backend (`registry`) and the frontend read it; the Rust `Tool` enum is gone and the
renderer dispatches on `format`. Installability is derived (`format ∈ IMPLEMENTED_FORMATS`), not stored.
Upstream guards drift with `scripts/check-tools.sh` (the no-jq twin of `check-divisions.sh`).

Remaining: the app bundles a baseline copy of `tools.json` — it should refresh from the catalog clone
at runtime (like the corpus), and aa's `check-tools.yml` CI workflow is still staged to land.

### Phase E: Multi-File Renderers

Implement special output shapes only after their path semantics are verified:

- Aider `CONVENTIONS.md`
- Windsurf `.windsurfrules`
- OpenClaw workspace directory
- Antigravity skill directories
- Kimi if current docs validate an installable custom-agent format

## Post-0.2.0 Punch List

Tracked inventory for the release after v0.2.0. Grouped by what unblocks each item.

### Auto-update

The updater UI, store, plugin, dedicated signing key, and publish tooling all ship.

1. **Endpoint — activated at the v0.2.0 release cut** (no longer post-0.2.0): host is `agency-agents-app.rubezhanin.app`
   (Caddy on umbp from `~/Sites/agency-agents/`), the v0.2.0 build runs without `SKIP_UPDATER`, and
   `publish-manifest.sh` rsyncs the signed manifest there. Resolved in `decisions.md` (2026-06-22).
2. **Opt-in automatic install** *(remaining)* — today the live path is check → notify → one-click Install;
   the user still clicks. Wire the inert "Install updates automatically" toggle to a real off-by-default
   setting that does background download → verify → install. Backend install/relaunch plumbing exists.
3. **Beta channel** — "Update channel: Stable" is a read-only placeholder; wire real channel selection.
4. **Bulk-install auto-deploy** (separate idea) — a subscription that auto-deploys newly-added catalog
   agents into a division/team/project. Distinct from app self-update.

### Catalog / registry

5. Refresh `tools.json` from the catalog clone at runtime instead of bundling a baseline copy.
6. Land `check-tools.yml` CI upstream (aa repo).
7. Foreign-sweep for nested `…/<dir>/SKILL.md` skills — CLI-installed Osaurus/Antigravity aren't
   auto-detected; app-installed ones are.

### New install targets (recognized → installable)

8. Multi-file renderers per Phase E (Aider, Windsurf, OpenClaw, Kimi).
9. Antigravity — *blocked on upstream* removing the non-deterministic `date_added`.

### Platform / packaging

10. Windows code signing — *blocked on a paid cert*.
11. Native runtime verification on Windows/Linux VMs (Phase C remainder).

### Accessibility (pre-existing)

12. Bulk-delete dialog focus management.
13. `role=menu` keyboard navigation.

### Longer horizon

14. Local-runtime system-prompt target (Ollama / LM Studio).

## Quality Gates

Before release:

```sh
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run check
npm run build
npm run build:phase-c
```

Renderer parity should be checked against the active AA clone:

```sh
AGENCY_AGENTS_PARITY_ROOT=/Users/michael/Software/AgentLand/agency-agents \
cargo test --manifest-path src-tauri/Cargo.toml upstream_convert_sh_is_byte_identical_for_transform_tools -- --ignored
```

## Definition Of Done For 1.0

- public docs describe Agency Agents, not the inherited source app
- app name, bundle ID, updater host, and release artifacts are consistent
- supported install paths have primary-source verification
- renderer parity passes for supported transform tools
- uninstall is recoverable for modified files
- macOS signed build is verified
- Windows/Linux builds are produced or explicitly marked unavailable
- Memory Bank task docs are updated after human approval

## v1.0 Roadmap (post-0.4.0)

Driven by the four technical reviews (see `docs/REVIEW-*.md` if present, or the four
attached TZs in the v0.4.0 milestone). Every item below is a single, reviewable PR.

### A. Foundation & Branding ✅ done in 0.4.0

- Re-brand to `rubezhanin/agency-agents-app` (bundle id, author, updater host, FUNDING, README).
- Drop macOS-only artefacts that don't make sense in a cross-platform fork (Liquid Glass icon source remains as a doc reference).
- Hermes plugin declared in `tools.json` (`hermes`, `installKind: "plugin"`).

### B. Hermes plugin (full)

- Rust renderer `src-tauri/src/render/hermes.rs` producing a directory per **[HERMES-PLUGIN.md](./HERMES-PLUGIN.md)**.
- CLI surface in the app: "Install as Hermes plugin" / "Stage for `hermes plugin install`…".
- Reconciliation: ledger records the plugin as one install with N child hashes; classify as `current | outdated | modified | removed | foreign`.
- UI: `DeploymentMatrix` shows a Hermes button for every persona; the modal renders the directory and offers install/remove/update.
- Tests: renderer fixture, schema validation, index sync, reconciliation states.

### C. CI/CD (cross-platform)

- New `ci.yml` — runs on every push and PR: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --lib`, `npm run check`, `npm run build`. Three runners (ubuntu, windows, macos).
- New `macos-build.yml` — release builds for `darwin-aarch64` + `darwin-x86_64`, signed + notarized.
- Coverage: `cargo-llvm-cov` → Codecov; minimum gate 70% on the Rust lib.
- Dependabot: weekly for npm and Cargo.
- Pre-commit: `gitleaks` for secret scanning; `prettier --check` for TS/Svelte.

### D. Architecture (decompose god modules)

Goal: every Rust file ≤ 15 KB, every Svelte component ≤ 10 KB. Public IPC unchanged.

| Current (KB) | New modules |
|--------------|-------------|
| `corpus/mod.rs` 85 | `corpus/{parser, indexer, git, cache, updater, runbooks}.rs` |
| `install/mod.rs` 66 | `install/{manager, ledger, reconciler, diff, backup, bulk, sandbox}.rs` |
| `commands/updater.rs` 51 | `updater/{checker, downloader, verifier, scheduler}.rs` |
| `commands/settings.rs` 47 | `settings/{model, validator, migrator, repository}.rs` |
| `ui.svelte.ts` 22 | `stores/{nav, theme, modal, layout, history}.ts` |
| `AgentsWorkspace.svelte` 35 | `components/agents/{Catalog, Filter, Detail, Actions}.svelte` |
| `ToolsView.svelte` 40 | `components/tools/{List, Detail, BulkActions, VersionBadge}.svelte` |

Also: introduce a thin `domain` layer (no Tauri, no tokio) and an `application` layer (use cases), so logic is testable without the Tauri runtime.

### E. Features (the long tail)

- **Event-driven sync** — backend emits `ledger-updated` / `install-progress`; frontend listens and applies optimistic updates. No more poll-after-invoke.
- **Pre-flight agent validation** — regex scan (`rm -rf`, `curl | sh`, `sudo`, reads of `.env`); flag as "High Risk" and require explicit Acknowledge.
- **Schema migrations** — every JSON file (settings, ledger, tools.json cache) carries a `schema_version`; chain migrations run on load.
- **Path sandbox** — `fs::sandbox::resolve_safe_path(root, input)` rejects any path whose canonical form escapes the root. Wire into every install/import/export/backup entry point.
- **Virtual scrolling** — `svelte-virtual-list` for the agent catalog and tools list (>50 items).
- **a11y** — focus trap in modals, `role=listbox` in command palette, full keyboard nav, ARIA labels on every icon button, color contrast ≥ 4.5:1, `axe-core` in CI.
- **Structured logging on frontend** — replace `catch { /* ignore */ }` with a `logError('event', ctx)` call into a local file.
- **Rust ↔ TS type generation** — `ts-rs` or a proc-macro that emits `src/lib/types.generated.ts` from `src-tauri/src/types.rs`; CI fails if drift is detected.
- **Rollback / Time machine** — UI timeline of an agent's history; "Rollback to v1.2" restores from `~/.agency-agents/backups/`.
- **Beta release channel** — `prerelease` flag in `updater.json`; opt-in toggle in Settings.
- **Coverage report** — Codecov badge in README.

### F. Tooling & DX

- `cargo-deny` (licenses + advisories) wired into CI; `deny.toml` strict on copyleft.
- `cargo audit` + `npm audit` weekly.
- `rust-toolchain.toml` pinning stable + components (clippy, rustfmt).
- Conventional commits + `release-please` to cut releases automatically.
- ADR (Architecture Decision Records) in `docs/adr/` for every major choice.

### v1.0 Definition of Done (additive to the 0.x list above)

- [ ] Hermes plugin is installable and reconciled as documented in HERMES-PLUGIN.md.
- [ ] CI is green on ubuntu + windows + macos on every PR.
- [ ] Coverage ≥ 70% on `src-tauri/src/`, ≥ 60% on `src/lib/stores/`.
- [ ] No file > 15 KB in `src-tauri/src/`, no Svelte file > 10 KB in `src/lib/components/`.
- [ ] Every persisted JSON has a `schema_version` and migrator.
- [ ] axe-core CI run passes on every PR.
- [ ] Zero `catch { /* ignore */ }` without a `logError(...)` call.
- [ ] `cargo clippy -- -D warnings` and `cargo fmt --check` are required checks.
- [ ] `HERMES-PLUGIN.md` and `docs/HERMES-PLUGIN.md` exist and the schema validator passes against them.
- [ ] Beta channel toggle works end-to-end (manifest → updater → install).
- [ ] At least one migration has been tested by introducing a fake v0.4 ledger and observing it upgrade to v1.0 in CI.
