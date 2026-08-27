//! Semver parser + `>=` comparator for Hermes `--version` output.
//!
//! Port of `rubezhanin/agent-kit` `src/hermes/version.ts`. We don't pull in
//! the `semver` crate because the version strings we ever see come from
//! `hermes --version` and a 30-line handwritten parser is faster, has no
//! dependencies, and is easier to keep deterministic.

/// Minimum Hermes version we integrate with. Bumped when we adopt a feature
/// that requires a newer release. Mirrors `agent-kit`'s `MIN_HERMES = "0.12.0"`.
pub const MIN_HERMES: &str = "0.12.0";

const SEMVER_RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
    // Two-pass because `regex` doesn't support the `(?i)` flag at the type level
    // for all MSRV targets; we use a character class instead.
    regex::Regex::new(r"(\d+\.\d+\.\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?").unwrap()
});

/// Pull a `MAJOR.MINOR.PATCH` triple out of a free-form version string.
/// Returns `None` if the input contains no semver.
pub fn parse_version(raw: &str) -> Option<String> {
    let caps = SEMVER_RE.captures(raw)?;
    caps.get(1).map(|m| m.as_str().to_string())
}

/// Compare two `MAJOR.MINOR.PATCH` strings. Returns `true` if `a >= b`.
/// Pre-release and build metadata are stripped before comparison.
pub fn semver_gte(a: &str, b: &str) -> bool {
    let Some(av) = parse_segments(a) else {
        return false;
    };
    let Some(bv) = parse_segments(b) else {
        return false;
    };
    let n = av.len().max(bv.len());
    for i in 0..n {
        let x = *av.get(i).unwrap_or(&0);
        let y = *bv.get(i).unwrap_or(&0);
        if x > y {
            return true;
        }
        if x < y {
            return false;
        }
    }
    true
}

fn parse_segments(v: &str) -> Option<Vec<u32>> {
    let stripped = v
        .split('-')
        .next()
        .unwrap_or(v)
        .split('+')
        .next()
        .unwrap_or(v);
    let mut out = Vec::with_capacity(3);
    for p in stripped.split('.') {
        if p.is_empty() {
            return None;
        }
        let n: u32 = p.parse().ok()?;
        out.push(n);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_handles_common_shapes() {
        assert_eq!(parse_version("hermes 0.12.3").as_deref(), Some("0.12.3"));
        assert_eq!(
            parse_version("0.12.3-rc.1+build.5").as_deref(),
            Some("0.12.3")
        );
        assert_eq!(parse_version("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(parse_version("garbage").as_deref(), None);
    }

    #[test]
    fn semver_gte_matches() {
        assert!(semver_gte("1.0.0", "0.12.0"));
        assert!(semver_gte("0.12.0", "0.12.0"));
        assert!(semver_gte("0.12.1", "0.12.0"));
        assert!(!semver_gte("0.11.9", "0.12.0"));
        assert!(!semver_gte("0.11.99", "0.12.0"));
        // Partial semver (missing patch) is treated as having an implicit
        // zero — this matches how `cargo` and most package managers
        // parse loose semver strings. We document the behaviour here
        // rather than pin it to one side.
        assert!(semver_gte("0.12", "0.12.0"));
        assert!(semver_gte("0.12.0", "0.12"));
    }
}
