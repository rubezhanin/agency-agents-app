//! Hermes CLI probe.
//!
//! Port of `rubezhanin/agent-kit` `src/hermes/probe.ts`. We:
//! 1. Try `which("hermes")` first.
//! 2. Fall back to `scan_beyond_path` if PATH didn't yield anything.
//! 3. Run `<bin> --version`, `config path`, `kanban --help`, `profile list`
//!    to populate the rest of the probe record.
//!
//! Every subprocess runs with `CREATE_NO_WINDOW` on Windows to avoid the
//! flashing terminal windows some users hit on the Dashboard (issue #84
//! upstream).

use std::path::PathBuf;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use ts_rs::TS;

use crate::hermes::scan::{scan_beyond_path, ScanOptions};
use crate::hermes::version::{semver_gte, MIN_HERMES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "lowercase")]
pub enum ProbeSource {
    Path,
    Scan,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/types.generated.ts")]
#[serde(rename_all = "camelCase")]
pub struct HermesProbe {
    pub found: bool,
    pub path: Option<PathBuf>,
    pub source: ProbeSource,
    pub version: Option<String>,
    pub meets_minimum: bool,
    pub minimum: String,
    pub config_path: Option<String>,
    pub kanban_available: bool,
    pub profiles: Vec<String>,
    pub stderr_tail: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ProbeOptions {
    /// Override the PATH lookup; primarily for tests.
    pub path_override: Option<PathBuf>,
    /// Override the candidate list (instead of `scan_beyond_path`); tests.
    pub scan_candidates: Option<Vec<PathBuf>>,
    /// Override the homedir; tests.
    pub homedir: Option<PathBuf>,
    /// Override the platform string; tests.
    pub platform: Option<String>,
    /// Skip the (slow) `profile list` call. Useful for the dashboard tile.
    pub skip_profile_list: bool,
}

#[derive(Debug)]
pub struct ProbeInputs {
    pub hermes_path: Option<PathBuf>,
    pub source: ProbeSource,
    pub version: Option<String>,
    pub config_path: Option<String>,
    pub kanban_available: bool,
    pub profiles: Vec<String>,
    pub stderr_tail: Option<String>,
    pub minimum: String,
}

pub fn build_probe(input: ProbeInputs) -> HermesProbe {
    let found = input.hermes_path.is_some();
    let meets_minimum = match (&input.version, &input.minimum) {
        (Some(v), m) => semver_gte(v, m),
        _ => false,
    };
    HermesProbe {
        found,
        path: input.hermes_path,
        source: if found {
            input.source
        } else {
            ProbeSource::Missing
        },
        version: input.version,
        meets_minimum,
        minimum: input.minimum,
        config_path: input.config_path,
        kanban_available: input.kanban_available,
        profiles: input.profiles,
        stderr_tail: input.stderr_tail,
    }
}

pub async fn probe_hermes(opts: ProbeOptions) -> HermesProbe {
    // 1. PATH
    if let Some(p) = opts.path_override.clone().or_else(which_hermes) {
        return probe_at(p, ProbeSource::Path, opts, None).await;
    }

    // 2. Scan-beyond-path
    let candidates = opts.scan_candidates.clone().unwrap_or_else(|| {
        scan_beyond_path(&ScanOptions {
            homedir: opts.homedir.clone(),
            platform: opts.platform.clone(),
            env: None,
        })
        .into_iter()
        .map(|c| c.path)
        .collect()
    });
    for c in candidates {
        if let Some(version) = hermes_version(&c).await {
            return probe_at(c, ProbeSource::Scan, opts, Some(version)).await;
        }
    }

    build_probe(ProbeInputs {
        hermes_path: None,
        source: ProbeSource::Missing,
        version: None,
        config_path: None,
        kanban_available: false,
        profiles: Vec::new(),
        stderr_tail: None,
        minimum: MIN_HERMES.to_string(),
    })
}

async fn probe_at(
    path: PathBuf,
    source: ProbeSource,
    opts: ProbeOptions,
    precomputed_version: Option<String>,
) -> HermesProbe {
    let version = match precomputed_version {
        Some(v) => Some(v),
        None => hermes_version(&path).await,
    };
    let config = hermes_config_path(&path).await;
    let kanban = hermes_kanban_available(&path).await;
    let profiles = if !opts.skip_profile_list && kanban.available {
        hermes_profile_list(&path).await
    } else {
        Vec::new()
    };
    build_probe(ProbeInputs {
        hermes_path: Some(path),
        source,
        version,
        config_path: config.path,
        kanban_available: kanban.available,
        profiles,
        stderr_tail: kanban.stderr_tail.or(config.stderr_tail),
        minimum: MIN_HERMES.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Subprocess helpers
// ---------------------------------------------------------------------------

fn which_hermes() -> Option<PathBuf> {
    which("hermes")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in std::env::var_os("PATHEXT")
            .as_deref()
            .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
            .unwrap_or_default()
        {
            let candidate = dir
                .join(name)
                .with_extension(ext.to_string_lossy().trim_start_matches('.'));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        // On non-Windows, the bare name is fine.
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn base_command(bin: &std::path::Path) -> Command {
    let mut cmd = Command::new(bin);
    // Suppress the flashing-terminal-window bug on Windows (issue #84).
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

pub(crate) async fn hermes_version(bin: &std::path::Path) -> Option<String> {
    let out = base_command(bin).arg("--version").output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    crate::hermes::version::parse_version(s.trim())
}

pub(crate) async fn hermes_config_path(bin: &std::path::Path) -> ConfigPathResult {
    let out = base_command(bin).args(["config", "path"]).output().await;
    match out {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let lines: Vec<&str> = stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            match lines.first() {
                Some(first) => ConfigPathResult {
                    path: Some(first.to_string()),
                    stderr_tail: None,
                },
                None => ConfigPathResult {
                    path: None,
                    stderr_tail: None,
                },
            }
        }
        Ok(o) => ConfigPathResult {
            path: None,
            stderr_tail: Some(tail(&o.stderr)),
        },
        Err(_) => ConfigPathResult {
            path: None,
            stderr_tail: None,
        },
    }
}

pub(crate) async fn hermes_kanban_available(bin: &std::path::Path) -> KanbanResult {
    let out = base_command(bin).args(["kanban", "--help"]).output().await;
    match out {
        Ok(o) if o.status.success() => KanbanResult {
            available: true,
            stderr_tail: None,
        },
        Ok(o) => KanbanResult {
            available: false,
            stderr_tail: Some(tail(&o.stderr)),
        },
        Err(_) => KanbanResult {
            available: false,
            stderr_tail: None,
        },
    }
}

pub(crate) async fn hermes_profile_list(bin: &std::path::Path) -> Vec<String> {
    let out = base_command(bin).args(["profile", "list"]).output().await;
    let Ok(o) = out else { return Vec::new() };
    if !o.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Default)]
pub(crate) struct ConfigPathResult {
    pub path: Option<String>,
    pub stderr_tail: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct KanbanResult {
    pub available: bool,
    pub stderr_tail: Option<String>,
}

fn tail(buf: &[u8]) -> String {
    let s = String::from_utf8_lossy(buf);
    if s.len() > 200 {
        s[s.len() - 200..].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hermes::version::parse_version;

    #[test]
    fn build_probe_marks_meets_minimum_correctly() {
        let p = build_probe(ProbeInputs {
            hermes_path: Some(PathBuf::from("/usr/local/bin/hermes")),
            source: ProbeSource::Path,
            version: Some("1.2.3".into()),
            config_path: None,
            kanban_available: false,
            profiles: vec![],
            stderr_tail: None,
            minimum: "0.12.0".into(),
        });
        assert!(p.found);
        assert!(p.meets_minimum);

        let p2 = build_probe(ProbeInputs {
            hermes_path: None,
            source: ProbeSource::Missing,
            version: None,
            config_path: None,
            kanban_available: false,
            profiles: vec![],
            stderr_tail: None,
            minimum: "0.12.0".into(),
        });
        assert!(!p2.found);
        assert!(!p2.meets_minimum);
        assert!(matches!(p2.source, ProbeSource::Missing));
    }

    #[test]
    fn parse_version_extracts_from_realistic_strings() {
        assert_eq!(
            parse_version("hermes 0.12.3 (darwin x86_64)").as_deref(),
            Some("0.12.3")
        );
        assert_eq!(parse_version("v0.12.0").as_deref(), Some("0.12.0"));
    }
}
