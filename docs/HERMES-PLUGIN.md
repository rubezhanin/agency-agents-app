# Hermes Plugin — `agency-agents-router`

> **Status:** v1.2.0. Working contract between Agency Agents and the [`hermes`](https://hermes-agent.dev) CLI.
> Source of truth: this document + `src-tauri/src/render/hermes.rs` + `src-tauri/data/tools.json` → `hermes` entry.
> The plugin's `manifest.yaml` is **a strict superset of the upstream `agent-kit.manifest.schema.json`** ([rubezhanin/agent-kit `schema/agent-kit.manifest.schema.json`](https://github.com/rubezhanin/agent-kit/blob/main/schema/agent-kit.manifest.schema.json)) — every field of the upstream schema is preserved, and the plugin adds Hermes-plugin-specific fields under `plugin_meta`.
> Open questions are flagged with **[TBD]**.

Since v1.1.0 the plugin family supports **multi-plugin routing** — the
canonical `agency-agents-router` plus any number of custom plugins
named by the user (one per division, one per persona-set, etc.). The
renderer signature is now `render_named_plugin(agents, sources,
catalog_ref, app_version, plugin_id, plugin_label)`; the install
directory is `~/.hermes/plugins/<plugin_id>/`. The pre-flight check
(`hermes_preflight`) and the aggregated health snapshot
(`hermes_health`) document the runtime readiness of the install path;
see [§9 Pre-flight](#9-pre-flight) and [§10 Health snapshot](#10-health-snapshot).

## 1. What this is

Agency Agents ships a **plugin-style installer** for the `hermes` CLI. Unlike the rest of the supported tools (which are one-file-per-agent renderers), Hermes consumes a **directory** under `~/.hermes/plugins/agency-agents-router/`. The directory contains:

- a `manifest.yaml` describing the plugin (kit + plugin metadata);
- a top-level `SKILL.md` that teaches Hermes how to **route** a request to the right agent persona;
- one `skills/<slug>.md` per declared agent — the rendered personas, identical byte-for-byte to the catalog source.

The user's `hermes` CLI then loads the plugin and uses the router skill to dispatch. Removing the directory uninstalls the plugin.

## 2. Plugin directory layout

```text
~/.hermes/plugins/agency-agents-router/
├── manifest.yaml                    # kit + plugin metadata (see §3)
├── SKILL.md                         # the router skill — main entry point
├── router/
│   └── ROUTER.md                    # routing rules (catalog → persona) — optional, for human readers
└── skills/                          # one .md per declared agent (matches manifest.agents[].id)
    ├── frontend-architect.md
    ├── backend-engineer.md
    ├── devops-specialist.md
    └── ...                          # exactly one per manifest.agents[].id, no more, no less
```

Installed by either:

- **App** — `InstallModal` → "Install as Hermes plugin" (the renderer copies the whole directory to `~/.hermes/plugins/agency-agents-router/`).
- **CLI** — `hermes plugin install /path/to/agency-agents-router` (the renderer emits the directory to a staging path the user passes, then the user runs the command).
- **Reconciliation** — detects the plugin as a *single installable unit* with N child skills.

## 3. `manifest.yaml` schema (v1)

The manifest has two top-level sections: **`plugin_meta`** (Hermes-plugin concerns) and the **upstream `agent-kit.manifest` fields at the root** (`schema_version`, `id`, `display_name`, `agents`, `relationships`, `install_modes`, etc., as defined by `agent-kit.manifest.schema.json`).

```yaml
# ~/.hermes/plugins/agency-agents-router/manifest.yaml
# Validated by schema/agent-kit.manifest.schema.json plus the plugin_meta block below.
schema_version: 1                 # agent-kit schema version (currently 1)
id: agency-agents-router          # kit id; matches ^[a-z][a-z0-9_-]{0,63}$
display_name: Agency Agents Router
description: Routes the rubezhanin/agency-agents catalog personas into Hermes skills
privacy: open                     # local-only | team-internal | open

plugin_meta:                      # Agency-Agents-specific extension
  schema_version: 1               # plugin-meta sub-schema (independent of agent-kit)
  name: agency-agents-router      # must equal kit id above
  version: 0.4.0                  # mirrors the Agency Agents app version
  author: Yuri Shvets
  homepage: https://github.com/rubezhanin/agency-agents-app
  license: MIT
  type: router                    # [TBD] Hermes plugin kinds — confirm with hermes-kit
  entry: SKILL.md                 # the skill hermes loads first
  catalog:
    source: github:rubezhanin/agency-agents
    ref: <git-sha-or-tag>         # frozen at install time; reconciliation compares against it
    agents: 14                    # number of agents included in this build

agents:                           # required by agent-kit schema; mirrors plugin_meta.agents list
  - id: frontend-architect
    display_name: Frontend Architect
    role: Frontend architect for React/TypeScript SPAs
    workspace: frontend
    memory_scope: []              # private | shared:<name>
    skills: [frontend-architect]
  - id: backend-engineer
    display_name: Backend Engineer
    role: Backend engineer for Rust/Go/Node APIs
    workspace: backend
    memory_scope: []
    skills: [backend-engineer]
  - id: devops-specialist
    display_name: DevOps Specialist
    role: CI/CD, infra-as-code, observability
    workspace: devops
    memory_scope: []
    skills: [devops-specialist]
  # ... one entry per included agent

relationships:
  edges:                          # routing topology inside the plugin
    - { from: hermes-router, to: frontend-architect, kind: routes-to }
    - { from: hermes-router, to: backend-engineer, kind: routes-to }
    - { from: hermes-router, to: devops-specialist, kind: routes-to }

shared_resources: []              # no shared resources in v1

install_modes:
  routing: kanban                 # kanban | direct | wiki
  auto_install_hermes: false      # the user explicitly runs `hermes plugin install`
```

### 3.1. Field rules

| Field | Required | Type | Rule |
|-------|----------|------|------|
| `schema_version` (root) | yes | int | Currently `1`. Bump on any breaking change in the upstream agent-kit fields. |
| `id` | yes | kebab regex | `^[a-z][a-z0-9_-]{0,63}$`. Must equal `plugin_meta.name`. |
| `display_name` | yes | string | 1–80 chars. |
| `privacy` | yes | enum | `local-only` (default) \| `team-internal` \| `open`. |
| `agents` | yes | array | ≥ 1 item, each `{id, display_name, role, workspace, memory_scope[], skills[]}`. `id` matches the upstream regex. |
| `relationships.edges` | no | array | `routes-to` \| `reviews` \| `shares-with` \| `escalates-to`. |
| `install_modes.routing` | yes | enum | `kanban` (default) \| `direct` \| `wiki`. |
| `plugin_meta.schema_version` | yes | int | Independent of the root schema version. Currently `1`. |
| `plugin_meta.version` | yes | semver | Mirrors the Agency Agents app version that produced the plugin. |
| `plugin_meta.entry` | yes | rel path | Always `SKILL.md` in v1. |
| `plugin_meta.catalog.ref` | yes | git ref | Frozen at install; reconciliation compares bytes against the same ref. |
| `plugin_meta.type` | yes | enum | `router` in v1. **[TBD]** confirm with hermes-kit. |

## 4. `SKILL.md` (the router)

The router is a single Markdown file with YAML frontmatter that hermes loads as the entry skill:

```markdown
---
name: agency-agents-router
description: Route a user request to the right agent persona from the agency-agents catalog. Use this skill whenever a user describes a coding, design, ops, or content task and the best-fit persona isn't obvious.
---

# Agency Agents Router

You are the **router** for the [Agency Agents](https://github.com/rubezhanin/agency-agents)
catalog inside Hermes. Your job is to read the user's request, pick the right persona from
the skills in this plugin, and answer as that persona.

## Routing rules

1. Read the user's task carefully. Identify the *primary* domain (frontend, backend, infra, etc.).
2. Match the domain to a persona in `skills/`. Prefer the most specific match.
3. **Adopt the persona's voice** — read the matched `skills/<slug>.md` and answer as if you are that agent.
4. If no clear match exists, ask one short clarifying question.
5. Never combine two personas in one answer. Pick one; switch only when the user asks.

## Persona index

<!-- This block is regenerated by the app on every install. Do not hand-edit. -->
- `frontend-architect` → `skills/frontend-architect.md`
- `backend-engineer` → `skills/backend-engineer.md`
- `devops-specialist` → `skills/devops-specialist.md`
- ...
<!-- end generated block -->
```

The router skill is **generated** by the renderer — the persona index block is rebuilt on every install so it matches `manifest.agents` exactly.

## 5. Per-agent skills (`skills/<slug>.md`)

Each file is the rendered persona, **byte-identical** to the catalog source. The renderer MUST NOT add, remove, or rewrite content here — the persona's voice is the persona's voice. Reconciliation compares each file's bytes against the catalog source hash recorded at install time.

## 6. Reconciliation

A Hermes plugin is **one logical install** with **N child files**. The install ledger records:

```json
{
  "tool": "hermes",
  "scope": "user",
  "dest": ".hermes/plugins/agency-agents-router",
  "manifest_hash": "<sha256 of manifest.yaml>",
  "router_hash": "<sha256 of SKILL.md>",
  "skills": {
    "frontend-architect.md": "<sha256>",
    "backend-engineer.md": "<sha256>",
    "...": "..."
  },
  "catalog_ref": "<git-sha at install>",
  "app_version": "0.4.0"
}
```

Reconciliation classifies:

| State | Rule |
|-------|------|
| `current` | All N child hashes match the catalog source at `catalog_ref`. |
| `outdated` | Catalog moved; new versions exist; ledger hashes are stale. |
| `modified` | At least one file's bytes differ from its recorded hash. |
| `removed` | Directory or any required file is missing. |
| `foreign` | Directory present, but no ledger entry — user dropped it manually. |

The update path is: regenerate the entire directory (with new `catalog_ref`), back up the old one, atomic rename. This is the multi-file analog of the per-file overwrite used elsewhere.

## 7. CLI surface (in this app, not the `hermes` CLI)

The app does **not** shell out to `hermes`. It writes the directory directly to `~/.hermes/plugins/agency-agents-router/`. If the user prefers the CLI:

```sh
# Stage the plugin to a temp dir
# (App calls the renderer to a user-chosen staging path, then:)
hermes plugin install /tmp/agency-agents-router
```

The app surfaces a "Stage for `hermes plugin install`…" button in the Hermes install modal that writes to a user-picked directory.

## 8. Tests

- **Renderer unit test** — emits the canonical plugin from a fixture catalog; compares against the byte-identical expected directory in `src-tauri/src/render/hermes/test_fixtures/`.
- **Schema validation** — every emitted `manifest.yaml` parses against the schema in §3.
- **Index sync** — every `agents` entry in `manifest.yaml` has a matching `skills/<slug>.md`; the `skills/` directory has **no** files outside that list.
- **Reconciliation** — modified/removed/foreign states classify correctly against a fs-fixture.

## 9. Pre-flight

The app runs a pre-flight check before the user clicks "Install as
Hermes plugin" (and on every Settings → Hermes tile mount, via the
`hermes_preflight` IPC). The check is **informational only** — the
install buttons stay live even when a check fails, with a soft
warning rendered as the destructive-callout in the modal.

Five independent checks land in a single `HermesPreflight` response:

| id                  | what it tests                                                            | blocking |
|---------------------|--------------------------------------------------------------------------|----------|
| `hermes-cli`        | `hermes` binary on PATH or in a common install location; version ≥ `0.12.0` | no       |
| `hermes-kanban`     | `hermes kanban --help` succeeds (sub-feature presence)                  | no       |
| `node-runtime`      | `node` (or `bun`) on PATH, version string parses                         | no       |
| `home-writable`     | can create a temp file under `$HOME`                                     | yes      |
| `install-target`    | `~/.hermes/plugins/<plugin_id>/` is missing, or a writable directory     | yes      |

The `HermesPreflight { ready, checks, checkedAt, home }` is rendered
in the UI as a colour-coded checklist (green / yellow / red pill per
row, "X issues found" headline, expandable detail + remediation
strings). The `ready` summary is true when no row has `fail`; the
UI uses it for the banner.

## 10. Health snapshot

A separate `hermes_health` IPC bundles the probe, the pre-flight,
and a scan of `~/.hermes/plugins/` into a single round-trip and a
single timestamp. The Settings → Hermes tile polls it on a 60s
`setInterval` (started in `onMount`, cancelled in `onDestroy`).

`HermesHealthSnapshot { overall, probe, preflight, plugins, checkedAt }`
plus a derived `HermesHealthStatus`:

- **ok** — CLI on PATH, meets minimum, home writable, no warnings.
- **degraded** — CLI missing or outdated, OR a non-blocking warn
  fired (kanban missing, node missing, etc.). The user can still
  install — the renderer doesn't shell out to `hermes`.
- **down** — a blocking check failed (home not writable, install
  target conflicts). The install buttons remain enabled so the
  user can stage to a custom directory, but the badge is red.

The tile shows the overall as a colour-coded pill (green / yellow /
red) and the `checkedAt` ISO timestamp beside it.

## 11. Multi-plugin routing

Since v1.1.0 the install path is parametric: the user passes a
kebab-case `pluginId` (matching `[a-z0-9-]{1,64}`) and a human
`pluginLabel` to `hermes_install` / `hermes_stage`. The renderer
mirrors both into the plugin's `manifest.yaml` (`id:` and
`display_name:`) and into the router skill's `name:` / `# Heading`.

Canonical `agency-agents-router` is the default and is what
`hermes_install` writes when no id is supplied. Custom plugins land
in `~/.hermes/plugins/<plugin_id>/` and coexist with the canonical
one; `hermes_list_plugins` reads them back as a per-row
`{ pluginId, label, path, agentCount, isCanonical }` table for the
Settings → Hermes "Installed plugins" card.

Use cases: ship the `engineering` division as `engineering-team`,
the `growth` division as `growth-team`, etc. — Hermes sees them as
independent plugins with distinct router skills.

## 12. Open questions

- **[TBD]** Hermes plugin kinds — `router` is the proposed value. Confirm with `hermes-kit`.
- **[TBD]** Should `manifest.yaml` support per-agent hooks (e.g. `triggers`, `model_preference`)? Wait for hermes-kit v0.x to land a stable manifest spec.
- **[TBD]** Router `ROUTER.md` is not in the directory layout above — drop or keep?
- **[TBD]** When the user uninstalls via the app, should we run `hermes plugin remove …` (slower, but hermes may be caching)? Decision: direct fs remove; recommend restart of `hermes`.
- **[TBD]** Should the multi-plugin routing path also write a Hermes journal entry (mirroring the install journal) so a crashed install can be recovered on next launch? The current per-plugin install goes through `install_to` which is already atomic; only the in-flight write is at risk.

## 13. Versioning

| App version | Plugin manifest version | Notes                                                       |
|-------------|-------------------------|-------------------------------------------------------------|
| 0.4.0       | 1                       | First Hermes-aware release.                                 |
| 1.1.0       | 1                       | Multi-plugin routing: `render_named_plugin(...)`.           |
| 1.2.0       | 1                       | `hermes_preflight` + `hermes_health` (pre-flight + 60s poll).|

The manifest `schema_version` is independent of the app version. App `version` is recorded so users can see which build of the app produced their plugin.
