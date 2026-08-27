# Agency Agents Philosophy

Agency Agents exists because useful AI-agent catalogs should be easy to inspect, install, and maintain without hiding what is being written to a developer's machine.

The app is intentionally:

- local-first
- deterministic
- inspectable
- reversible
- respectful of user agency
- aligned with open agent tooling

## 1. The Catalog Remains The Source

The app does not replace the [`agency-agents`](https://github.com/rubezhanin/agency-agents) repo. It gives that repo a native app surface.

The catalog source is either bundled, managed locally, or chosen by the user. Agent source remains plain Markdown. Tool-specific outputs are deterministic renders of that source.

## 2. The App Tracks What The Tools Do Not

Most AI coding tools read agent files from config directories, but they do not expose a shared install database.

Agency Agents fills that gap with a local ledger:

- what agent was installed
- where it was written
- which tool and scope it belongs to
- which corpus hash it came from
- which rendered bytes the app produced

That ledger is the basis for update detection, drift protection, recoverable uninstall, and project-level visibility.

## 3. Determinism Over Magic

An install should be explainable:

1. read canonical source
2. render a known tool format
3. write to a known destination
4. record source and rendered hashes
5. reconcile by re-rendering and comparing bytes

No hidden prompt mutation, no runtime LLM dependency, no invisible file stamping.

## 4. Local-First By Default

The app should be useful without accounts or background services.

Network use is explicit:

- refresh the `agency-agents` catalog
- sign in to GitHub
- call GitHub-backed features
- check for app updates

Core browsing, install tracking, reconciliation, and uninstall behavior operate locally.

## 5. No Telemetry

Agency Agents does not collect:

- analytics
- click tracking
- installed-agent inventory
- tool usage metrics
- device fingerprints
- behavioral profiles

There are no hidden analytics SDKs and no product-improvement event stream.

## 6. Security Is Product Behavior

The app writes files into developer tooling directories. That demands conservative behavior.

Rules:

- deterministic renderers only
- no arbitrary shell bridge from the frontend
- no production mock data
- no destructive uninstall of modified files without backup
- no network path without a visible feature reason
- no pretending an unverified tool path is authoritative

## 7. Respect Tool-Specific Reality

Different tools have different agent formats. Some are one-file-per-agent. Some are project-scoped. Some use aggregate files. Some require CLI registration.

The app should expose that honestly instead of forcing every integration into the same shape.

For unsupported or unverified tools, the UI should say so.

## 8. Quiet Native UX

The interface should favor:

- dense information
- clear state
- keyboard navigation
- low ceremony
- visible file paths
- recoverable operations

Over:

- gamification
- engagement loops
- opaque recommendations
- ornamental dashboards

Agency Agents should make the user's agent setup easier to understand, not harder.
