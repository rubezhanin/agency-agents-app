//! Hermes CLI location scan.
//!
//! Port of `rubezhanin/agent-kit` `src/hermes/scan.ts` to Rust. If `hermes`
//! isn't on PATH, this builds a candidate list of well-known install
//! locations and returns the ones that exist and are executable.
//!
//! We don't shell out here — just check the filesystem. The version probe
//! happens separately in `probe.rs`.

use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanOrigin {
    HomeLocal,
    HomeBin,
    Cargo,
    HermesBin,
    HomebrewOpt,
    HomebrewUsrLocal,
    Snap,
    Flatpak,
    WinLocalPrograms,
    WinLocalAppData,
    WinProgramFiles,
}

#[derive(Debug, Clone)]
pub struct ScanCandidate {
    pub path: PathBuf,
    /// Provenance — useful for the UI's "where was this found?" hint and
    /// for parity tests. The runtime probe doesn't read it; we keep it
    /// populated so the structured log line stays useful.
    #[allow(dead_code)]
    pub origin: ScanOrigin,
}

#[derive(Debug, Default, Clone)]
pub struct ScanOptions {
    pub homedir: Option<PathBuf>,
    pub platform: Option<String>,
    pub env: Option<std::collections::HashMap<String, String>>,
}

const HOME_BIN_NAMES: &[&str] = &["hermes", "hermes-agent"];

/// Resolve the user's home directory in a way that works on every platform
/// we ship to, without pulling the `directories` crate (we already have
/// `dirs` in Cargo.toml — but `dirs::home_dir` is a deprecated name; the
/// stable API is `dirs::home_dir()` which still works).
fn default_homedir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn file_exists(p: &std::path::Path) -> bool {
    p.is_file()
}

fn is_executable(p: &std::path::Path, platform: &str) -> bool {
    if !file_exists(p) {
        return false;
    }
    #[cfg(unix)]
    {
        if platform == "win32" {
            return true;
        }
        if let Ok(meta) = std::fs::metadata(p) {
            return meta.permissions().mode() & 0o111 != 0;
        }
        return false;
    }
    #[cfg(not(unix))]
    {
        let _ = platform;
        true
    }
}

fn build_candidates(
    home: &std::path::Path,
    platform: &str,
    env: &std::collections::HashMap<String, String>,
) -> Vec<ScanCandidate> {
    let mut out = Vec::new();
    for name in HOME_BIN_NAMES {
        out.push(ScanCandidate {
            path: home.join(".local").join("bin").join(name),
            origin: ScanOrigin::HomeLocal,
        });
        out.push(ScanCandidate {
            path: home.join("bin").join(name),
            origin: ScanOrigin::HomeBin,
        });
        out.push(ScanCandidate {
            path: home.join(".cargo").join("bin").join(name),
            origin: ScanOrigin::Cargo,
        });
        out.push(ScanCandidate {
            path: home.join(".hermes").join("bin").join(name),
            origin: ScanOrigin::HermesBin,
        });
    }
    if platform == "darwin" {
        for name in HOME_BIN_NAMES {
            out.push(ScanCandidate {
                path: PathBuf::from("/opt/homebrew/bin").join(name),
                origin: ScanOrigin::HomebrewOpt,
            });
            out.push(ScanCandidate {
                path: PathBuf::from("/usr/local/bin").join(name),
                origin: ScanOrigin::HomebrewUsrLocal,
            });
        }
    }
    if platform == "linux" {
        for name in HOME_BIN_NAMES {
            out.push(ScanCandidate {
                path: PathBuf::from("/snap/bin").join(name),
                origin: ScanOrigin::Snap,
            });
        }
    }
    if platform == "win32" {
        let local = env
            .get("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        let program_files = env
            .get("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
        for name in HOME_BIN_NAMES {
            let exe = format!("{name}.exe");
            out.push(ScanCandidate {
                path: local.join("Programs").join("hermes-agent").join(&exe),
                origin: ScanOrigin::WinLocalPrograms,
            });
            out.push(ScanCandidate {
                path: local.join("hermes-agent").join(&exe),
                origin: ScanOrigin::WinLocalAppData,
            });
            out.push(ScanCandidate {
                path: program_files.join("hermes-agent").join(&exe),
                origin: ScanOrigin::WinProgramFiles,
            });
        }
    }
    out
}

pub fn scan_beyond_path(opts: &ScanOptions) -> Vec<ScanCandidate> {
    let platform = opts
        .platform
        .clone()
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    let home = opts.homedir.clone().unwrap_or_else(default_homedir);
    let env: std::collections::HashMap<String, String> = opts
        .env
        .clone()
        .unwrap_or_else(|| std::env::vars().collect());

    let mut found = Vec::new();
    for c in build_candidates(&home, &platform, &env) {
        if is_executable(&c.path, &platform) {
            found.push(c);
        }
    }

    // Flatpak on Linux
    if platform == "linux" {
        let root = std::path::Path::new("/var/lib/flatpak/exports/bin");
        if root.is_dir() {
            if let Ok(entries) = std::fs::read_dir(root) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.to_lowercase().contains("hermes") && is_executable(&e.path(), &platform)
                    {
                        found.push(ScanCandidate {
                            path: e.path(),
                            origin: ScanOrigin::Flatpak,
                        });
                    }
                }
            }
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(home: &std::path::Path, platform: &str) -> ScanOptions {
        ScanOptions {
            homedir: Some(home.to_path_buf()),
            platform: Some(platform.to_string()),
            env: Some(Default::default()),
        }
    }

    #[test]
    fn scan_returns_empty_when_nothing_matches() {
        let dir = tempfile::tempdir().unwrap();
        let candidates = scan_beyond_path(&opts(dir.path(), "linux"));
        assert!(candidates.is_empty());
    }

    #[test]
    fn scan_finds_home_local_hermes() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join(".local").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let p = bin.join("hermes");
        std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let candidates = scan_beyond_path(&opts(dir.path(), "linux"));
        assert!(candidates.iter().any(|c| c.path == p));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn build_candidates_includes_macos_paths() {
        let home = std::path::Path::new("/Users/test");
        let env = std::collections::HashMap::new();
        let c = build_candidates(home, "darwin", &env);
        let paths: Vec<String> = c
            .iter()
            .map(|x| x.path.to_string_lossy().to_string())
            .collect();
        assert!(paths.iter().any(|p| p.contains("/opt/homebrew/bin/hermes")));
        assert!(paths.iter().any(|p| p.contains("/usr/local/bin/hermes")));
    }

    #[test]
    fn build_candidates_includes_windows_paths() {
        let home = std::path::Path::new("C:\\Users\\test");
        let mut env = std::collections::HashMap::new();
        env.insert(
            "LOCALAPPDATA".into(),
            "C:\\Users\\test\\AppData\\Local".into(),
        );
        env.insert("ProgramFiles".into(), "C:\\Program Files".into());
        let c = build_candidates(home, "win32", &env);
        let paths: Vec<String> = c
            .iter()
            .map(|x| x.path.to_string_lossy().to_string())
            .collect();
        assert!(paths
            .iter()
            .any(|p| p.contains("hermes-agent") && p.contains("hermes.exe")));
    }
}
