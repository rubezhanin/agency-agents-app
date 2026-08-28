//! Deterministic per-tool agent renderers + destination-path
//! resolution, split into submodules.
//!
//! Module layout:
//!
//! - `helpers` — pure helpers: `sha256_hex`, `slugify`, `source_field`,
//!   `source_body`, `resolve_opencode_color`, `toml_escape`,
//!   `unsupported`. IO-free, easy to test.
//! - `dispatch` — registry-backed surface helpers: `supports_user`,
//!   `supports_project`, `scope_for`, `label`.
//! - `output` — `output_slug`, `render`, `render_with_hash`. The
//!   load-bearing deterministic conversion.
//! - `dests` — destination-path resolution.
//! - `hermes` — Hermes plugin renderer (directory producer, lives
//!   outside the one-file-only `render` dispatch).
//! - `tests` — integration tests for the conversion + dests layers.
//!
//! Ports `agency-agents` `scripts/convert.sh`. Every renderer is a PURE
//! function of `(Agent, raw source)` — no timestamps, no randomness,
//! stable key order — so `rendered_hash` is reproducible. That
//! reproducibility is the load-bearing requirement for
//! `install::reconcile`: we identify an installed file as "ours" by
//! re-rendering its slug for its tool and matching bytes.

pub mod dests;
pub mod dispatch;
pub mod helpers;
pub mod hermes;
pub mod output;

#[cfg(test)]
mod tests;

// Public re-exports — the rest of the app imports these by their
// previous, flat paths (`render::render`, `render::dests`,
// `render::slugify`, etc.). `slugify` is in `helpers` but is a public
// API other modules (e.g. `corpus`) consume.
pub use dests::dests;
pub use dispatch::{label, scope_for, supports_project, supports_user};
pub use helpers::{sha256_hex, slugify};
#[allow(unused_imports)] // re-exported for downstream callers in install/tests
pub use output::{output_slug, render, render_with_hash};
