//! Hermes CLI detection and probing.
//!
//! Port of `rubezhanin/agent-kit` `src/hermes/{probe,scan,version}.ts` to
//! Rust. The user-facing flow:
//!
//! 1. `which("hermes")` (PATH) — preferred.
//! 2. If missing, scan plausible install locations (`~/.local/bin/hermes`,
//!    `~/.hermes/bin/hermes`, Homebrew, snap, Windows Program Files, etc.).
//! 3. For each candidate, run `<bin> --version` and parse semver.
//! 4. Probe `--version`, `config path`, `kanban --help`, `profile list`.
//! 5. Build a `HermesProbe` record the UI can render in Tools / Settings.
//!
//! All subprocess calls go through `tokio::process::Command` and are
//! `CREATE_NO_WINDOW` on Windows so the dashboard doesn't flash terminal
//! windows (issue #84 in the upstream tracker).

pub mod probe;
pub mod scan;
pub mod version;

// Re-exports the public surface. Internal types (`ProbeInputs`,
// `ProbeSource`, `ScanCandidate`, `ScanOrigin`) stay inside their
// modules — callers go through the public functions and `HermesProbe`.
pub use probe::{probe_hermes, HermesProbe, ProbeOptions};
