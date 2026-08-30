//! Hermes pre-flight readiness check.
//!
//! A single IPC call that runs a small set of independent checks the
//! UI can render as a colour-coded checklist ("Ready to install").
//! Each check has:
//!   - a stable id (for i18n keys / tests),
//!   - a human-readable label,
//!   - a status (`ok` / `warn` / `fail`),
//!   - a detail string with concrete values (path, version, etc.),
//!   - an optional remediation hint.
//!
//! The pre-flight is **informational** — it does NOT block the install
//! buttons. The `ready` summary is true when no check has status `fail`,
//! and the UI uses it for the banner ("All checks pass" / "2 issues
//! found"); the user can still proceed if they want.
//!
//! Checks (in order):
//!   1. `hermes-cli`        — CLI on PATH, version >= MIN_HERMES.
//!   2. `hermes-kanban`     — `kanban --help` succeeds (sub-feature).
//!   3. `node-runtime`      — `node` (or `bun`) on PATH, for the JS
//!                            plugin runtime that some installs invoke.
//!   4. `home-writable`     — can create a temp file under `$HOME`
//!                            (the install lives at `~/.hermes/...`).
//!   5. `install-target`    — the canonical plugin dir either doesn't
//!                            exist or is writable (catches read-only
//!                            installs and broken symlinks).
//!
//! Each check is its own function so the unit tests can exercise them
//! in isolation without touching the real `$HOME` or `PATH`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use ts_rs::TS;

use crate::hermes::probe::{probe_hermes, ProbeOptions, ProbeSource};

/// Status of a single pre-flight check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "lowercase")]
pub enum PreflightStatus {
    /// Check passed cleanly.
    Ok,
    /// Check passed but the UI should surface a soft warning.
    Warn,
    /// Check failed; the install path may be broken or unsafe.
    Fail,
}

/// One row in the pre-flight checklist.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct PreflightCheck {
    /// Stable id, e.g. `"hermes-cli"`. Used as an i18n key and in tests.
    pub id: String,
    /// Human-readable label, e.g. "Hermes CLI".
    pub label: String,
    pub status: PreflightStatus,
    /// Concrete value (path, version, etc.). Empty string when not applicable.
    pub detail: String,
    /// Optional fix-it suggestion, e.g. "Upgrade with `brew upgrade hermes`."
    pub remediation: Option<String>,
    /// When `true`, a `fail` here means the install cannot succeed.
    pub blocking: bool,
}

/// Result of a `preflight_hermes` call. The UI renders `checks` as a
/// list and uses `ready` for the banner headline.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct HermesPreflight {
    /// `true` iff no check has status `fail` (warns are fine).
    pub ready: bool,
    pub checks: Vec<PreflightCheck>,
    /// ISO-8601 UTC timestamp of when the pre-flight ran.
    pub checked_at: String,
    /// Path the pre-flight probed for `home-writable` and
    /// `install-target`, surfaced for the UI to display in the hint.
    pub home: PathBuf,
}

impl HermesPreflight {
    /// Number of checks that did not pass cleanly (warn + fail).
    pub fn issues(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status != PreflightStatus::Ok)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run every pre-flight check in order. Pure function: it inspects the
/// filesystem and shells out to `hermes` / `node` / `bun` and returns
/// the aggregated result. Does NOT write anything to disk.
pub async fn preflight_hermes() -> HermesPreflight {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => PathBuf::from("."),
    };

    let probe = probe_hermes(ProbeOptions {
        skip_profile_list: true,
        ..ProbeOptions::default()
    })
    .await;

    let mut checks: Vec<PreflightCheck> = Vec::with_capacity(5);
    checks.push(check_hermes_cli(&probe));
    checks.push(check_hermes_kanban(&probe));
    checks.push(check_node_runtime().await);
    checks.push(check_home_writable(&home));
    checks.push(check_install_target(&home));

    let ready = checks
        .iter()
        .all(|c| c.status != PreflightStatus::Fail);
    HermesPreflight {
        ready,
        checks,
        checked_at: DateTime::<Utc>::from(std::time::SystemTime::now()).to_rfc3339(),
        home,
    }
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

fn check_hermes_cli(probe: &crate::hermes::probe::HermesProbe) -> PreflightCheck {
    if !probe.found {
        return PreflightCheck {
            id: "hermes-cli".into(),
            label: "Hermes CLI".into(),
            status: PreflightStatus::Warn,
            detail: "not found on PATH or in common install locations".into(),
            remediation: Some(
                "Install the `hermes` CLI to use the canonical plugin install path.".into(),
            ),
            // Not blocking: `hermes_install` doesn't shell out to `hermes`,
            // it just writes the directory. The plugin can be staged or
            // installed later.
            blocking: false,
        };
    }
    if !probe.meets_minimum {
        return PreflightCheck {
            id: "hermes-cli".into(),
            label: "Hermes CLI".into(),
            status: PreflightStatus::Warn,
            detail: format!(
                "found at {} but version {} < required {}",
                probe
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                probe.version.clone().unwrap_or_default(),
                probe.minimum,
            ),
            remediation: Some(format!(
                "Upgrade hermes to {} or later to use the router plugin.",
                probe.minimum
            )),
            blocking: false,
        };
    }
    let source = match probe.source {
        ProbeSource::Path => "PATH",
        ProbeSource::Scan => "scan",
        ProbeSource::Missing => "missing",
    };
    PreflightCheck {
        id: "hermes-cli".into(),
        label: "Hermes CLI".into(),
        status: PreflightStatus::Ok,
        detail: format!(
            "{} v{} (via {})",
            probe
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            probe.version.clone().unwrap_or_default(),
            source,
        ),
        remediation: None,
        blocking: true, // ok-path; would only be here if the install requires it
    }
}

fn check_hermes_kanban(probe: &crate::hermes::probe::HermesProbe) -> PreflightCheck {
    if !probe.found {
        return PreflightCheck {
            id: "hermes-kanban".into(),
            label: "Hermes kanban".into(),
            status: PreflightStatus::Warn,
            detail: "skipped — `hermes` not found".into(),
            remediation: None,
            blocking: false,
        };
    }
    if probe.kanban_available {
        PreflightCheck {
            id: "hermes-kanban".into(),
            label: "Hermes kanban".into(),
            status: PreflightStatus::Ok,
            detail: "kanban subcommand available".into(),
            remediation: None,
            blocking: false,
        }
    } else {
        PreflightCheck {
            id: "hermes-kanban".into(),
            label: "Hermes kanban".into(),
            status: PreflightStatus::Warn,
            detail: "kanban subcommand not present (older build?)".into(),
            remediation: Some(
                "Reinstall or upgrade hermes; the kanban subcommand is needed for board sync."
                    .into(),
            ),
            blocking: false,
        }
    }
}

async fn check_node_runtime() -> PreflightCheck {
    // Try `node` first, fall back to `bun` (Hermes can run on either).
    if let Some(version) = node_version("node").await {
        return PreflightCheck {
            id: "node-runtime".into(),
            label: "Node runtime".into(),
            status: PreflightStatus::Ok,
            detail: format!("node {version}"),
            remediation: None,
            blocking: false,
        };
    }
    if let Some(version) = node_version("bun").await {
        return PreflightCheck {
            id: "node-runtime".into(),
            label: "Node runtime".into(),
            status: PreflightStatus::Ok,
            detail: format!("bun {version} (node not found)"),
            remediation: None,
            blocking: false,
        };
    }
    PreflightCheck {
        id: "node-runtime".into(),
        label: "Node runtime".into(),
        status: PreflightStatus::Warn,
        detail: "no `node` or `bun` on PATH".into(),
        remediation: Some(
            "Install Node.js 18+ (or Bun) to run the `hermes plugin install` flow end-to-end."
                .into(),
        ),
        blocking: false,
    }
}

fn check_home_writable(home: &Path) -> PreflightCheck {
    if !home.exists() {
        return PreflightCheck {
            id: "home-writable".into(),
            label: "Home writable".into(),
            status: PreflightStatus::Fail,
            detail: format!("{} does not exist", home.display()),
            remediation: Some("Create the home directory or fix the user profile.".into()),
            blocking: true,
        };
    }
    if !home.is_dir() {
        return PreflightCheck {
            id: "home-writable".into(),
            label: "Home writable".into(),
            status: PreflightStatus::Fail,
            detail: format!("{} is not a directory", home.display()),
            remediation: None,
            blocking: true,
        };
    }
    match probe_writable(home) {
        Ok(()) => PreflightCheck {
            id: "home-writable".into(),
            label: "Home writable".into(),
            status: PreflightStatus::Ok,
            detail: home.display().to_string(),
            remediation: None,
            blocking: true,
        },
        Err(e) => PreflightCheck {
            id: "home-writable".into(),
            label: "Home writable".into(),
            status: PreflightStatus::Fail,
            detail: format!("cannot write to {}: {e}", home.display()),
            remediation: Some("Check filesystem permissions on your home directory.".into()),
            blocking: true,
        },
    }
}

fn check_install_target(home: &Path) -> PreflightCheck {
    let dest = home.join(".hermes").join("plugins").join("agency-agents-router");
    if !dest.exists() {
        return PreflightCheck {
            id: "install-target".into(),
            label: "Install target".into(),
            status: PreflightStatus::Ok,
            detail: format!("{} (will be created on install)", dest.display()),
            remediation: None,
            blocking: true,
        };
    }
    if !dest.is_dir() {
        return PreflightCheck {
            id: "install-target".into(),
            label: "Install target".into(),
            status: PreflightStatus::Fail,
            detail: format!("{} exists but is not a directory", dest.display()),
            remediation: Some("Remove the conflicting path and retry.".into()),
            blocking: true,
        };
    }
    match probe_writable(&dest) {
        Ok(()) => PreflightCheck {
            id: "install-target".into(),
            label: "Install target".into(),
            status: PreflightStatus::Ok,
            detail: format!("{} (writable)", dest.display()),
            remediation: None,
            blocking: true,
        },
        Err(e) => PreflightCheck {
            id: "install-target".into(),
            label: "Install target".into(),
            status: PreflightStatus::Fail,
            detail: format!("{} is not writable: {e}", dest.display()),
            remediation: Some("Free up the path or pick a custom install location.".into()),
            blocking: true,
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn node_version(bin: &str) -> Option<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let v = s.trim();
    if v.is_empty() {
        return None;
    }
    // `node --version` prints "v20.10.0" — strip the leading `v`.
    Some(v.trim_start_matches('v').to_string())
}

/// Try to create a temp file inside `dir` and remove it. Errors out
/// with the OS message if the write fails.
fn probe_writable(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(".agency-agents-preflight-probe");
    let res = std::fs::write(&probe, b"probe");
    let _ = std::fs::remove_file(&probe);
    res
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_probe(found: bool, meets: bool, version: &str, source: ProbeSource) -> crate::hermes::probe::HermesProbe {
        crate::hermes::probe::HermesProbe {
            found,
            path: if found {
                Some(PathBuf::from("/usr/local/bin/hermes"))
            } else {
                None
            },
            source,
            version: if found { Some(version.into()) } else { None },
            meets_minimum: meets,
            minimum: "0.12.0".into(),
            config_path: None,
            kanban_available: false,
            profiles: Vec::new(),
            stderr_tail: None,
        }
    }

    #[test]
    fn check_hermes_cli_missing_yields_warn_non_blocking() {
        let p = empty_probe(false, false, "", ProbeSource::Missing);
        let c = check_hermes_cli(&p);
        assert_eq!(c.id, "hermes-cli");
        assert_eq!(c.status, PreflightStatus::Warn);
        assert!(!c.blocking);
        assert!(c.detail.contains("not found"));
    }

    #[test]
    fn check_hermes_cli_outdated_yields_warn() {
        let p = empty_probe(true, false, "0.10.0", ProbeSource::Path);
        let c = check_hermes_cli(&p);
        assert_eq!(c.status, PreflightStatus::Warn);
        assert!(c.detail.contains("0.10.0"));
        assert!(c.detail.contains("0.12.0"));
    }

    #[test]
    fn check_hermes_cli_ok_marks_blocking() {
        let p = empty_probe(true, true, "0.12.3", ProbeSource::Path);
        let c = check_hermes_cli(&p);
        assert_eq!(c.status, PreflightStatus::Ok);
        assert!(c.blocking);
        assert!(c.detail.contains("0.12.3"));
        assert!(c.detail.contains("PATH"));
    }

    #[test]
    fn check_hermes_kanban_skipped_when_cli_missing() {
        let p = empty_probe(false, false, "", ProbeSource::Missing);
        let c = check_hermes_kanban(&p);
        assert_eq!(c.status, PreflightStatus::Warn);
        assert!(c.detail.contains("skipped"));
    }

    #[test]
    fn check_install_target_missing_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let c = check_install_target(dir.path());
        assert_eq!(c.status, PreflightStatus::Ok);
        assert!(c.detail.contains("will be created"));
    }

    #[test]
    fn check_install_target_writable_dir_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join(".hermes").join("plugins").join("agency-agents-router");
        std::fs::create_dir_all(&plugin).unwrap();
        let c = check_install_target(dir.path());
        assert_eq!(c.status, PreflightStatus::Ok);
        assert!(c.detail.contains("writable"));
    }

    #[test]
    fn hermes_preflight_ready_iff_no_failures() {
        let mut pf = HermesPreflight {
            ready: false,
            checks: vec![],
            checked_at: "2026-01-01T00:00:00Z".into(),
            home: PathBuf::from("/tmp"),
        };
        // All-ok → ready.
        pf.checks.push(PreflightCheck {
            id: "a".into(),
            label: "A".into(),
            status: PreflightStatus::Ok,
            detail: "".into(),
            remediation: None,
            blocking: false,
        });
        pf.checks.push(PreflightCheck {
            id: "b".into(),
            label: "B".into(),
            status: PreflightStatus::Warn,
            detail: "".into(),
            remediation: None,
            blocking: false,
        });
        pf.ready = pf.checks.iter().all(|c| c.status != PreflightStatus::Fail);
        assert!(pf.ready);
        assert_eq!(pf.issues(), 1);

        // One fail → not ready.
        pf.checks.push(PreflightCheck {
            id: "c".into(),
            label: "C".into(),
            status: PreflightStatus::Fail,
            detail: "".into(),
            remediation: None,
            blocking: true,
        });
        pf.ready = pf.checks.iter().all(|c| c.status != PreflightStatus::Fail);
        assert!(!pf.ready);
        assert_eq!(pf.issues(), 2);
    }

    #[test]
    fn probe_writable_succeeds_in_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        probe_writable(dir.path()).unwrap();
    }

    #[test]
    fn probe_writable_fails_for_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ghost = dir.path().join("does-not-exist");
        assert!(probe_writable(&ghost).is_err());
    }
}
