//! Corpus subsystem (Phase 1) — the maintained copy of the agency-agents
//! repo that the whole app reads from.
//!
//! ## Source of truth (systemPatterns.md §1)
//!
//! ```text
//! <app_data_dir>/
//! ├── corpus/                 our maintained copy of the agency-agents repo
//! │   └── <category>/<slug>.md
//! └── state/
//!     └── corpus-index.json   slug → CorpusEntry (hashes, category, version)
//! ```
//!
//! - **Seed**: a baseline corpus ships inside the app bundle
//!   (`resources/corpus-baseline/<category>/<slug>.md`). On first run it is
//!   copied to `<app_data_dir>/corpus/` so the app works offline.
//! - **Refresh** ([`corpus_refresh`]): fetch the GitHub tarball
//!   `https://codeload.github.com/rubezhanin/agency-agents/tar.gz/refs/heads/main`,
//!   extract the category dirs over the working copy, and rebuild
//!   `corpus-index.json`. No runtime git dependency.
//!
//! ## Determinism (contracts.md §E)
//!
//! `corpus-index.json` is keyed by a `BTreeMap` so its serialization has a
//! stable key order. The three per-agent hashes are SHA-256 of canonical
//! byte regions of the source `.md` (see [`parse`]). Nothing in the index
//! carries a timestamp; the only timestamp is [`CorpusMeta::fetched_at`],
//! which lives in a separate meta file, not the index.

mod parse;
pub mod source;
pub mod tarball;
mod categories;
mod paths;
mod catalog_detect;
mod catalog;
mod catalog_ipc;
pub mod runbooks;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager};

use self::categories::{
    bundled_division_meta, bundled_division_slugs, DivisionsFile, DIVISIONS_FILENAME,
};
use crate::error::AppError;
use crate::state::AppState;
// Re-export `state_dir` so external callers (`install::ledger_path`,
// anything else that needs `<app_data>/state`) can use the natural
// `corpus::state_dir(...)` form without having to know the helper
// lives in `corpus::paths`. `pub(crate)` keeps it internal — these
// are *our* filesystem layout helpers, not part of the crate's public
// API surface. The other helpers (`corpus_dir`, `index_path`, `meta_path`,
// `catalog_source_path`) are only consumed inside the `corpus` module
// itself for now, so they stay un-re-exported.
pub(crate) use self::paths::state_dir;



// `run_git` / `git_available` / `has_git_dir` live in `catalog_detect`
// and are consumed by `catalog_status` / `catalog_check_updates` which
// are still in this file. When those IPCs move out (stage C of the
// decomposition) this import goes away.

// ---------- Constants ----------

/// The division set for the active catalog = the keys of its `divisions.json`
/// (the canonical division truth the agency-agents repo declares, shared with
/// the CLI installer and the linters). Read the active root's file when present
/// (a clone, or the seeded baseline once it carries one); otherwise fall back to
/// the bundled floor (`agency-categories.json`, itself a mirror of the catalog's
/// `divisions.json`).
///
/// Deriving from `divisions.json` rather than parsing `convert.sh`'s `AGENT_DIRS`
/// fixes a class of drift: a top-level dir that ISN'T a declared division — e.g.
/// `strategy/`, which holds NEXUS playbooks/runbooks with no agent frontmatter —
/// is never surfaced as a division OR scanned as one, and a newly-declared
/// division (e.g. `healthcare`) appears the moment the catalog carries it, with
/// no app-side list to keep in sync. This value doubles as the division list AND
/// the set of directories the indexer scans for agents; both are correct because
/// every agent-bearing dir is a declared division and no non-division dir holds
/// agents (enforced upstream by `check-divisions.sh`'s `NON_DIVISION_DIRS`).
pub(super) fn discover_categories(root: &Path) -> Vec<String> {
    let meta = std::fs::read_to_string(root.join(DIVISIONS_FILENAME))
        .ok()
        .and_then(|raw| serde_json::from_str::<DivisionsFile>(&raw).ok())
        .map(|f| f.divisions)
        .unwrap_or_else(bundled_division_meta);
    let mut cats: Vec<String> = meta.into_keys().collect();
    cats.sort();
    cats
}

/// Extract the `AGENT_DIRS=( … )` bash array body from a shell script's text.
/// Returns the ordered, de-duplicated directory names, or `None` if the array
/// isn't found. Pure string work so it's unit-testable without the filesystem.


// Corpus struct + impl + build/load/persist/scan moved to `catalog.rs`
// (stage B of the decomposition). See that module for the data shape,
// the cold-path build, and the tarball refresh.

// Heuristic: does `root` hold an agency-agents catalog? True if it has the
// repo tooling or at least one of the canonical category dirs with agents.
// Shared with `corpus::catalog_detect` (the candidate walker) and with the
// `catalog_source_set` IPC (validates a user-chosen path). Kept in `mod`
// (not in `catalog`) because it's a one-screen filesystem check that
// several call sites already imported via `super::looks_like_catalog`.
pub(super) fn looks_like_catalog(root: &Path) -> bool {
    if root.join("scripts").join("convert.sh").exists() {
        return true;
    }
    self::categories::bundled_division_meta()
        .keys()
        .any(|c| root.join(c).is_dir())
}

pub(super) use self::catalog::is_empty_dir;
pub(crate) use self::catalog::MAX_TARBALL_BYTES;
pub(crate) use crate::corpus::catalog::{
    download_corpus_tarball, read_source, refresh, resolve_active, Corpus,
};

// ---------- Common helpers (shared with install + IPC) ----------

/// Resolve the bundled baseline dir from the Tauri resource dir. In
/// dev the resources live under the crate; in a bundled app they're
/// inside the `.app`. Tauri's `resource_dir()` resolves both.
fn baseline_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    let res = app.path().resource_dir().map_err(|e| AppError::Internal {
        message: format!("resolve resource_dir: {e}"),
    })?;
    Ok(res.join("resources").join("corpus-baseline"))
}

/// Resolve the per-app data dir via Tauri's path resolver (honors the
/// bundle id `app.rubezhanin.agency-agents-app`).
pub(crate) fn app_data_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path().app_data_dir().map_err(|e| AppError::Internal {
        message: format!("resolve app_data_dir: {e}"),
    })
}

/// Ensure the in-memory corpus is built + memoized on `AppState`, then
/// return the shared `Arc`. First call seeds (if needed), parses, and
/// persists the index; subsequent calls are a cheap cache read.
pub(crate) async fn ensure_corpus(
    app: &AppHandle,
    state: &AppState,
) -> Result<Arc<Corpus>, AppError> {
    // Hold the cache lock across the ENTIRE init — check, seed, parse, store.
    // The frontend fires corpus_list + corpus_categories (+ corpus_status)
    // concurrently on mount; a released-lock double-check would let each run
    // `seed_from_baseline` at once, racing on the same `<file>.tmp` paths
    // (rename → ENOENT). Serializing the first load is correct and cheap:
    // it happens once, and every later call is a fast locked cache read.
    let mut cached = state.corpus_cache.lock().await;
    if let Some(c) = cached.as_ref() {
        return Ok(Arc::clone(c));
    }
    let adir = app_data_dir(app)?;
    let bdir = baseline_dir(app)?;
    let corpus = Arc::new(resolve_active(&adir, &bdir).await);
    *cached = Some(Arc::clone(&corpus));
    Ok(corpus)
}




// Tauri IPC commands live in `catalog_ipc.rs` (stage C of the
// decomposition). Re-export them here so the rest of the codebase keeps
// using the `corpus::corpus_status(...)` form without a code change.
// Glob import (rather than an explicit list) so the
// `__tauri_command_name_*` and `__cmd__*` companions that `#[tauri::command]`
// generates for each IPC are also re-exported — the
// `tauri::generate_handler!` macro in `lib.rs` resolves those names
// through the `corpus::` path.
pub(crate) use crate::corpus::catalog_ipc::*;
// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use self::tarball::parse_agent_dirs;
    use super::runbooks::RunbooksFile;
    use super::*;
    // The few candidate-detection tests below stayed in this file (they share
    // fixture state with the broader corpus tests). The helpers they touch
    // moved to `catalog_detect` in stage A of the decomposition.
    use crate::corpus::catalog_detect::{candidate_for, quick_agent_count};
    // `build_from_dir` moved to `corpus::catalog` in stage B of the
    // decomposition. The corpus-build tests below stayed in this file
    // (they share the bundled-baseline fixture), so they import the
    // builder directly from the new home.
    use crate::corpus::catalog::build_from_dir;
    // The catalog-source / catalog-root helpers used by the
    // round-trip / build tests moved with the IPC commands in
    // stage C; import them directly from `corpus::source` so
    // the tests don't depend on the `catalog_ipc` re-export.
    use crate::corpus::source::{catalog_root, load_catalog_source, save_catalog_source};
    use crate::types::CatalogSource;

    fn write_agent(dir: &Path, category: &str, slug: &str, name: &str, body: &str) {
        let cat = dir.join(category);
        std::fs::create_dir_all(&cat).unwrap();
        let content = format!("---\nname: {name}\ndescription: d\n---\n{body}\n");
        std::fs::write(cat.join(format!("{slug}.md")), content).unwrap();
    }

    #[tokio::test]
    async fn build_indexes_agents_in_stable_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // Write out of order across two categories.
        write_agent(dir, "engineering", "zeta", "Zeta", "z");
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "design", "mid", "Mid", "m");

        let corpus = build_from_dir(dir, "test", &discover_categories(dir))
            .await
            .unwrap();
        assert_eq!(corpus.count(), 3);
        // design < engineering, and within engineering alpha < zeta.
        let order: Vec<&str> = corpus.agents.iter().map(|a| a.slug.as_str()).collect();
        assert_eq!(order, vec!["mid", "alpha", "zeta"]);
    }

    #[tokio::test]
    async fn build_indexes_nested_agents() {
        // Real clones nest agents in subdirs (game-development/godot/<slug>.md).
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "flat-one", "Flat One", "x");
        let nested = dir.join("game-development").join("godot");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("godot-shader-developer.md"),
            "---\nname: Godot Shader Developer\ndescription: d\n---\nbody\n",
        )
        .unwrap();

        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();
        let nested_agent = corpus.get("godot-shader-developer");
        assert!(nested_agent.is_some(), "nested agent must be indexed");
        assert_eq!(
            nested_agent.unwrap().category,
            "game-development",
            "category is the top-level dir, not the subdir"
        );
        assert!(corpus.get("flat-one").is_some(), "flat agent still indexed");
    }

    #[tokio::test]
    async fn index_json_is_byte_stable_across_builds() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "design", "mid", "Mid", "m");

        let cats = discover_categories(dir);
        let a = build_from_dir(dir, "v", &cats)
            .await
            .unwrap()
            .index_json()
            .unwrap();
        let b = build_from_dir(dir, "v", &cats)
            .await
            .unwrap()
            .index_json()
            .unwrap();
        assert_eq!(a, b, "corpus-index.json must be deterministic");
    }

    #[tokio::test]
    async fn list_omits_body_get_includes_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "the persona body");
        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();

        let listed = corpus.list(None);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].body, "", "list view must omit body");

        let full = corpus.get("alpha").unwrap();
        assert!(
            full.body.contains("the persona body"),
            "get must include body"
        );
    }

    #[tokio::test]
    async fn list_filters_by_category() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "design", "mid", "Mid", "m");
        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();

        let eng = corpus.list(Some("engineering"));
        assert_eq!(eng.len(), 1);
        assert_eq!(eng[0].slug, "alpha");
    }

    #[tokio::test]
    async fn categories_returns_all_divisions_with_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "alpha", "Alpha", "a");
        write_agent(dir, "engineering", "beta", "Beta", "b");
        // No divisions.json in this tempdir → discover falls back to the bundled floor.
        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();

        let cats = corpus.categories();
        assert_eq!(cats.len(), 17, "all declared divisions always returned");
        let eng = cats.iter().find(|c| c.slug == "engineering").unwrap();
        assert_eq!(eng.count, 2);
        assert_eq!(eng.label, "Engineering");
        assert_eq!(eng.icon, "Code");
        // Empty category still present with count 0.
        let fin = cats.iter().find(|c| c.slug == "finance").unwrap();
        assert_eq!(fin.count, 0);
        // `healthcare` is a declared division (empty here, count 0). `strategy`
        // is NOT (it holds playbooks/runbooks, not agents) and `integrations` is
        // NOT (it's convert.sh output) — neither may appear as a division.
        let hc = cats.iter().find(|c| c.slug == "healthcare").unwrap();
        assert_eq!(hc.count, 0);
        assert!(
            !cats.iter().any(|c| c.slug == "strategy"),
            "strategy is not a division"
        );
        assert!(
            !cats.iter().any(|c| c.slug == "integrations"),
            "integrations is not a division"
        );
    }

    #[tokio::test]
    async fn non_agent_files_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_agent(dir, "engineering", "real", "Real", "x");
        // A README with no frontmatter.
        let cat = dir.join("engineering");
        std::fs::write(cat.join("README.md"), "# Examples\nnope\n").unwrap();
        // A workflow doc with no frontmatter.
        std::fs::write(cat.join("workflow.md"), "# Workflow\nnope\n").unwrap();

        let corpus = build_from_dir(dir, "v", &discover_categories(dir))
            .await
            .unwrap();
        assert_eq!(corpus.count(), 1);
        assert!(corpus.get("real").is_some());
        assert!(corpus.get("workflow").is_none());
    }

    #[tokio::test]
    async fn seed_then_build_round_trips() {
        let baseline = tempfile::tempdir().unwrap();
        write_agent(baseline.path(), "engineering", "alpha", "Alpha", "a");
        write_agent(baseline.path(), "design", "mid", "Mid", "m");

        let app_data = tempfile::tempdir().unwrap();
        let corpus = resolve_active(app_data.path(), baseline.path()).await;
        assert_eq!(corpus.count(), 2);
        // Working copy + index were written.
        assert!(self::paths::corpus_dir(app_data.path())
            .join("engineering/alpha.md")
            .exists());
        assert!(self::paths::index_path(app_data.path()).exists());
        assert!(self::paths::meta_path(app_data.path()).exists());
    }

    #[test]
    fn title_case_handles_hyphens() {
        assert_eq!(self::categories::title_case("game-development"), "Game Development");
        assert_eq!(self::categories::title_case("engineering"), "Engineering");
    }

    #[test]
    fn category_meta_resolves_from_bundled_json() {
        let bundled = self::categories::bundled_division_meta();
        let (label, icon, color) = self::categories::category_meta_from(&bundled, "engineering");
        assert_eq!(label, "Engineering");
        assert_eq!(icon, "Code");
        assert_eq!(color, "#3B82F6");
    }

    #[test]
    fn category_meta_falls_back_for_unknown_slug() {
        let bundled = self::categories::bundled_division_meta();
        let (label, icon, color) = self::categories::category_meta_from(&bundled, "made-up-division");
        assert_eq!(label, "Made Up Division");
        assert_eq!(icon, "Folder");
        assert_eq!(color, self::categories::default_division_color());
    }

    #[test]
    fn load_division_meta_missing_file_uses_bundled() {
        // First-run / pre-#592 clone: no divisions.json at the root → bundled.
        let root = tempfile::tempdir().unwrap();
        let meta = self::categories::load_division_meta(root.path());
        assert_eq!(meta.get("engineering").unwrap().color, "#3B82F6");
    }

    #[test]
    fn load_division_meta_overlays_catalog_divisions_json() {
        // A catalog divisions.json overrides a known division AND introduces a
        // brand-new one the bundled floor has never heard of (the whole point:
        // a new catalog division presents correctly without an app update).
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(DIVISIONS_FILENAME),
            r##"{ "divisions": {
                "engineering": { "label": "Engineering", "icon": "Cpu", "color": "#000000" },
                "robotics":    { "label": "Robotics",    "icon": "Bot", "color": "#FF00FF" }
            } }"##,
        )
        .unwrap();
        let meta = self::categories::load_division_meta(root.path());
        // Overridden from the catalog.
        let eng = meta.get("engineering").unwrap();
        assert_eq!((eng.icon.as_str(), eng.color.as_str()), ("Cpu", "#000000"));
        // Net-new division, present only in the catalog.
        assert_eq!(meta.get("robotics").unwrap().color, "#FF00FF");
        // A bundled division the catalog file omitted is retained (overlay, not replace).
        assert_eq!(meta.get("marketing").unwrap().label, "Marketing");
    }

    #[test]
    fn load_division_meta_malformed_file_uses_bundled() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(DIVISIONS_FILENAME), "{ not valid json ").unwrap();
        let meta = self::categories::load_division_meta(root.path());
        assert_eq!(meta.get("engineering").unwrap().color, "#3B82F6");
    }

    /// Parse the REAL bundled baseline corpus (not a synthetic tempdir) so a
    /// malformed real agent (bad frontmatter fence, missing `name`) fails CI
    /// rather than shipping. `cargo test` runs with cwd = crate root, so the
    /// relative resource path resolves. Divisions come from the bundled floor
    /// (`agency-categories.json`, a mirror of the catalog's `divisions.json`), so
    /// `strategy/` (playbooks/runbooks) is NOT a division and `integrations/`
    /// (convert.sh output) is NOT either. Counts are pinned to the agency-agents
    /// snapshot — bump them on a corpus refresh.
    #[tokio::test]
    async fn real_bundled_baseline_parses_completely() {
        let dir = Path::new("resources/corpus-baseline");
        if !dir.exists() {
            // Resources not present in this build context — skip rather than fail.
            return;
        }
        // Divisions come from the bundled floor (no divisions.json in the baseline).
        let categories = discover_categories(dir);
        assert!(
            !categories.iter().any(|c| c == "strategy"),
            "strategy is not a division"
        );
        assert!(
            !categories.iter().any(|c| c == "integrations"),
            "integrations is convert.sh output, not a division"
        );

        let corpus = build_from_dir(dir, "baseline-test", &categories)
            .await
            .unwrap();

        // 209 = 210 prior minus the lone `integrations/` artifact
        // (backend-architect-with-memory), which is convert.sh output, not a
        // catalog persona.
        assert_eq!(
            corpus.count(),
            209,
            "all bundled agent personas indexed (integrations excluded)"
        );

        // Every agent parsed real frontmatter: non-empty name + slug, real category.
        for a in &corpus.agents {
            assert!(!a.name.trim().is_empty(), "agent {} has empty name", a.slug);
            assert!(!a.slug.trim().is_empty(), "agent has empty slug");
            assert!(
                categories.contains(&a.category),
                "agent {} has unknown category {}",
                a.slug,
                a.category
            );
        }

        // Spot-check categories that nest agents in subdirs upstream — these are
        // the ones a flat seeding would silently undercount.
        let cats = corpus.categories();
        assert_eq!(cats.len(), 17, "17 declared divisions");
        let count_of = |slug: &str| {
            cats.iter()
                .find(|c| c.slug == slug)
                .map(|c| c.count)
                .unwrap_or(0)
        };
        assert_eq!(count_of("engineering"), 30);
        assert_eq!(count_of("specialized"), 46);
        // game-development nests agents in unity/, godot/, unreal-engine/ etc.
        // upstream; a flat seeding would silently undercount these.
        assert_eq!(
            count_of("game-development"),
            20,
            "nested game-dev agents included"
        );
        // strategy is NOT a division (playbooks/runbooks, no agent frontmatter),
        // so it never appears as one — regardless of what's on disk.
        assert!(
            !cats.iter().any(|c| c.slug == "strategy"),
            "strategy is not a division"
        );
        // healthcare IS a declared division; the bundled baseline predates its
        // agents, so it's present but empty (count 0) until a sync brings them in.
        assert_eq!(
            count_of("healthcare"),
            0,
            "healthcare present but empty in the stale baseline"
        );
    }

    #[test]
    fn parse_agent_dirs_reads_the_bash_array() {
        let script = r#"
# preamble
ALL_TOOLS=(claude-code copilot)
AGENT_DIRS=(
  academic design engineering   # inline comment ignored
  finance strategy
)
echo done
"#;
        let cats = parse_agent_dirs(script).unwrap();
        assert_eq!(
            cats,
            vec!["academic", "design", "engineering", "finance", "strategy"]
        );
        assert!(!cats.contains(&"integrations".to_string()));
    }

    #[test]
    fn parse_agent_dirs_none_when_absent() {
        assert!(parse_agent_dirs("nothing here").is_none());
    }

    #[tokio::test]
    async fn conversion_slug_resolves_filename_prefixed_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("engineering")).unwrap();
        std::fs::write(
            dir.join("engineering/engineering-frontend-developer.md"),
            "---\nname: Frontend Developer\ndescription: Builds UIs.\n---\nBody\n",
        )
        .unwrap();
        let corpus = build_from_dir(dir, "v", &["engineering".into()])
            .await
            .unwrap();

        let agent = corpus
            .get_by_conversion_slug("frontend-developer")
            .expect("convert.sh filename resolves");
        assert_eq!(agent.slug, "engineering-frontend-developer");
    }

    #[tokio::test]
    async fn catalog_source_persists_and_defaults_bundled() {
        let app_data = tempfile::tempdir().unwrap();
        // No file yet → default Bundled.
        assert_eq!(
            load_catalog_source(app_data.path()).await,
            CatalogSource::Bundled
        );

        let src = CatalogSource::Managed {
            path: "/Users/x/.agency-agents".into(),
        };
        save_catalog_source(app_data.path(), &src).await.unwrap();
        assert_eq!(load_catalog_source(app_data.path()).await, src);

        // catalog.json is valid camelCase-tagged JSON.
        let bytes = std::fs::read(self::paths::catalog_source_path(app_data.path())).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("\"kind\": \"managed\""),
            "tagged on kind: {text}"
        );
    }

    #[test]
    fn catalog_root_resolves_per_source() {
        let app_data = Path::new("/app/data");
        assert_eq!(
            catalog_root(app_data, &CatalogSource::Bundled),
            self::paths::corpus_dir(app_data)
        );
        assert_eq!(
            catalog_root(
                app_data,
                &CatalogSource::Managed {
                    path: "/home/x/.agency-agents".into()
                }
            ),
            PathBuf::from("/home/x/.agency-agents")
        );
        assert_eq!(
            catalog_root(
                app_data,
                &CatalogSource::UserClone {
                    path: "/src/aa".into(),
                    manage: true
                }
            ),
            PathBuf::from("/src/aa")
        );
    }

    #[test]
    fn looks_like_catalog_detects_tooling_or_categories() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            !looks_like_catalog(tmp.path()),
            "empty dir is not a catalog"
        );
        // A category dir is enough.
        std::fs::create_dir_all(tmp.path().join("engineering")).unwrap();
        assert!(looks_like_catalog(tmp.path()));
        // …or the tooling.
        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp2.path().join("scripts")).unwrap();
        std::fs::write(
            tmp2.path().join("scripts/convert.sh"),
            "AGENT_DIRS=(engineering)\n",
        )
        .unwrap();
        assert!(looks_like_catalog(tmp2.path()));
    }

    #[test]
    fn quick_count_and_candidate_from_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Not a catalog yet.
        assert!(candidate_for(root, "userClone").is_none());

        write_agent(root, "engineering", "a", "A", "x");
        write_agent(root, "engineering", "b", "B", "y");
        write_agent(root, "design", "c", "C", "z");
        std::fs::write(root.join("engineering/README.md"), "# readme").unwrap();

        assert_eq!(quick_agent_count(root), 3, "README excluded; 3 real agents");
        let cand = candidate_for(root, "userClone").unwrap();
        assert_eq!(cand.kind, "userClone");
        assert_eq!(cand.agent_count, 3);
        assert!(!cand.has_git, "no .git in this tempdir");
    }

    #[test]
    fn discover_categories_falls_back_to_bundled_floor_without_divisions_json() {
        let tmp = tempfile::tempdir().unwrap();
        let cats = discover_categories(tmp.path());
        // No divisions.json → the bundled floor (agency-categories.json) keys.
        assert_eq!(cats, self::categories::bundled_division_slugs());
        assert!(cats.contains(&"healthcare".to_string()) && cats.contains(&"gis".to_string()));
        assert!(
            !cats.contains(&"strategy".to_string()),
            "no phantom strategy division"
        );
    }

    #[test]
    fn discover_categories_reads_divisions_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(DIVISIONS_FILENAME),
            r##"{"divisions":{"healthcare":{"label":"Healthcare","icon":"Stethoscope","color":"#0D9488"},"engineering":{"label":"Engineering","icon":"Code","color":"#3B82F6"}}}"##,
        )
        .unwrap();
        // The active catalog's divisions.json is authoritative — its keys, sorted.
        let cats = discover_categories(tmp.path());
        assert_eq!(
            cats,
            vec!["engineering".to_string(), "healthcare".to_string()]
        );
        assert!(!cats.contains(&"strategy".to_string()));
    }

    #[test]
    fn runbooks_manifest_parses_and_defaults_empty() {
        let raw = r#"{"runbooks":[{"slug":"startup-mvp","title":"Startup MVP Build","mode":"NEXUS-Sprint","duration":"4-6 weeks","summary":"Idea to live.","doc":"strategy/runbooks/scenario-startup-mvp.md","roster":[{"group":"Core Team","activation":"always","agents":["agents-orchestrator","engineering-frontend-developer"]}]}]}"#;
        let file: RunbooksFile = serde_json::from_str(raw).unwrap();
        assert_eq!(file.runbooks.len(), 1);
        let rb = &file.runbooks[0];
        assert_eq!(rb.slug, "startup-mvp");
        assert_eq!(rb.mode, "NEXUS-Sprint");
        assert_eq!(rb.roster[0].agents.len(), 2);
        assert!(rb.roster[0]
            .agents
            .contains(&"engineering-frontend-developer".to_string()));
        // An absent `runbooks` key (bundled / no strategy/) parses to empty, not an error.
        let empty: RunbooksFile = serde_json::from_str("{}").unwrap();
        assert!(empty.runbooks.is_empty());
    }
}
