//! The `Corpus` data structure + the in-process build/load/persist/refresh
//! path. Split from `corpus/mod.rs` as stage B of the decomposition.
//!
//! ## What's here
//!
//! - The on-disk `corpus-meta.json` shape ([`StoredMeta`]) + its
//!   conversion to the public [`CorpusMeta`] type used by IPC.
//! - The in-memory [`Corpus`] struct (agents + index + category order
//!   + division presentation metadata) and the read-side methods on it
//!   ([`Corpus::list`], [`Corpus::get`], [`Corpus::entry`],
//!   [`Corpus::categories`], etc.).
//! - The cold-path build that turns a directory of `.md` files into a
//!   [`Corpus`]: [`resolve_active`] (seed + index + persist),
//!   [`build_from_dir`], [`seed_from_baseline`], [`empty_corpus`].
//! - The persistence helpers ([`persist`], [`load_stored_meta`]) and the
//!   tarball-driven refresh ([`refresh`], [`download_corpus_tarball`]).
//! - The small filesystem helpers ([`collect_md_files`], [`find_md_under`],
//!   [`is_empty_dir`], [`read_capped`]).
//!
//! ## Where the seam lives
//!
//! The data flows are: directory on disk -> `build_from_dir` -> [`Corpus`]
//! in memory -> [`persist`] (index + meta on disk). On the next launch
//! [`resolve_active`] re-reads the meta and rebuilds the in-memory tree.
//! IPC commands in `corpus::catalog_ipc` (stage C) call into this module
//! to materialise the read views.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::corpus::categories::{bundled_division_slugs, CategoryMetaRow};
use crate::corpus::parse;
use crate::corpus::source::{catalog_root, load_catalog_source};
use crate::corpus::tarball;
use crate::error::AppError;
use crate::types::{Agent, CatalogSource, Category, CorpusEntry, CorpusMeta};
use crate::util::fs::atomic_write;

// Sibling modules — imported so we can call `categories::xxx` /
// `paths::xxx` / `discover_categories` (which lives in `corpus::mod`)
// without a `self::` prefix.
use super::discover_categories;
use crate::corpus::categories;
use crate::corpus::paths;

// ---------- Tarball + scan constants ----------

/// GitHub `codeload` tarball for the live corpus. Streamed, gunzipped,
/// and unpacked on [`corpus_refresh`]. No git binary required.
const CORPUS_TARBALL_URL: &str =
    "https://codeload.github.com/rubezhanin/agency-agents/tar.gz/refs/heads/main";

/// User-Agent for the refresh fetch. Mirrors the catalog refresh style.
const USER_AGENT: &str = "agency-agents/0.1 (+https://github.com/rubezhanin/agency-agents)";

/// Whole-request timeout for the tarball fetch. The repo is small (a few
/// hundred small markdown files) so 60s is generous.
const REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Cap on the raw `tar.gz` response (defends against a hostile mirror).
/// The real tarball is well under 5 MiB; 32 MiB is large headroom.
pub const MAX_TARBALL_BYTES: u64 = 32 * 1024 * 1024;

/// Cap on a single decompressed agent `.md`. Personas run a few KiB;
/// 1 MiB is absurdly generous and still bounds memory.
const MAX_AGENT_BYTES: u64 = 1024 * 1024;

/// Version string recorded for the bundled baseline before any refresh
/// has resolved a commit SHA.
const BASELINE_VERSION: &str = "baseline";

// ---------- On-disk meta ----------


/// `corpus-meta.json` — top-level metadata for the working copy. Distinct
/// from the index (which is per-agent) so [`corpus_status`] can answer
/// "what version / how many / fetched when" with one small read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMeta {
    version: String,
    commit: Option<String>,
    fetched_at: String,
    count: u32,
}

impl From<StoredMeta> for CorpusMeta {
    fn from(m: StoredMeta) -> Self {
        CorpusMeta {
            version: m.version,
            commit: m.commit,
            fetched_at: m.fetched_at,
            count: m.count,
        }
    }
}

// ---------- In-memory corpus ----------

/// The parsed, in-memory corpus: every agent plus its index row, ordered
/// deterministically by `(category, slug)`. Memoized on `AppState` so the
/// hot read commands (`corpus_list` / `corpus_get` / `corpus_categories`)
/// never touch disk after the first build.
#[derive(Debug, Clone)]
pub struct Corpus {
    /// Agents in stable `(category, slug)` order. `Agent.body` is fully
    /// populated here; list views clone-and-clear it (see
    /// [`Corpus::list`]).
    pub(super) agents: Vec<Agent>,
    /// Index rows keyed by slug — `BTreeMap` so the serialized
    /// `corpus-index.json` has stable key order.
    index: BTreeMap<String, CorpusEntry>,
    /// The category directories this corpus was built from, in tooling order
    /// (from [`discover_categories`]). Drives the Discover grid so the tiles
    /// match the active catalog's actual divisions.
    category_order: Vec<String>,
    /// Division presentation metadata (label / icon / color) keyed by slug,
    /// resolved at build time: the catalog root's `divisions.json` overlaid on
    /// the bundled `agency-categories.json` floor (see [`load_division_meta`]).
    /// Carrying it on the corpus means `categories()` never touches disk and a
    /// catalog that ships a new division presents correctly without an app
    /// update.
    division_meta: BTreeMap<String, CategoryMetaRow>,
    meta: CorpusMeta,
}

impl Corpus {
    /// Number of indexed agents.
    pub fn count(&self) -> u32 {
        self.index.len() as u32
    }

    /// [`CorpusMeta`] for `corpus_status`.
    pub fn meta(&self) -> CorpusMeta {
        self.meta.clone()
    }

    /// List view — agents (optionally filtered to one `category`) with the
    /// `body` omitted to keep the IPC payload small (contracts.md §C).
    pub fn list(&self, category: Option<&str>) -> Vec<Agent> {
        self.agents
            .iter()
            .filter(|a| category.is_none_or(|c| a.category == c))
            .map(|a| Agent {
                body: String::new(),
                ..a.clone()
            })
            .collect()
    }

    /// Full agent (incl. body) by slug, or `None` if unknown.
    pub fn get(&self, slug: &str) -> Option<Agent> {
        self.agents.iter().find(|a| a.slug == slug).cloned()
    }

    /// Resolve a filename emitted by `convert.sh` back to the catalog's
    /// filename-based identity. Most upstream filenames include a division
    /// prefix while transformed installs use `slugify(frontmatter.name)`.
    pub fn get_by_conversion_slug(&self, slug: &str) -> Option<Agent> {
        self.agents
            .iter()
            .find(|a| crate::render::slugify(&a.name) == slug)
            .cloned()
    }

    /// Index row (hashes + category) by slug, for the install/reconcile layer.
    pub fn entry(&self, slug: &str) -> Option<CorpusEntry> {
        self.index.get(slug).cloned()
    }

    /// The active corpus version (from meta), used to stamp ledger records.
    pub fn version(&self) -> String {
        self.meta.version.clone()
    }

    /// Per-category counts in tooling order (from [`discover_categories`]).
    /// Label + icon + color come from [`Corpus::division_meta`] — the catalog's
    /// `divisions.json` overlaid on the bundled floor. Categories with zero
    /// agents are still returned so the Discover grid renders the full division
    /// set.
    pub fn categories(&self) -> Vec<Category> {
        let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
        for entry in self.index.values() {
            *counts.entry(entry.category.as_str()).or_default() += 1;
        }
        self.category_order
            .iter()
            .map(|slug| {
                let (label, icon, color) = categories::category_meta_from(&self.division_meta, slug);
                Category {
                    slug: slug.clone(),
                    label,
                    icon,
                    color,
                    count: counts.get(slug.as_str()).copied().unwrap_or(0),
                }
            })
            .collect()
    }

    /// Serialize the index to canonical pretty JSON. Stable key order
    /// (BTreeMap) → byte-identical output for an unchanged corpus.
    pub(super) fn index_json(&self) -> Result<Vec<u8>, AppError> {
        serde_json::to_vec_pretty(&self.index).map_err(|e| AppError::Internal {
            message: format!("serialize corpus-index.json: {e}"),
        })
    }
}

// ---------- Build / load ----------

/// Resolve the active corpus for the current process:
///
/// 1. Seed the working copy from the bundled baseline if `corpus/` is
///    empty (first run).
/// 2. Parse + index everything under `corpus/`.
/// 3. Write `corpus-index.json` + `corpus-meta.json` if they are missing
///    or stale (so reconciliation has the index on disk too).
///
/// `baseline_dir` is the bundled baseline resolved from the Tauri
/// resource dir (`resource_dir()/resources/corpus-baseline`). `Never`
/// panics: a fully empty or unreadable corpus yields an empty [`Corpus`]
/// with `count == 0` so the UI degrades to "no agents" rather than
/// failing to launch.
pub async fn resolve_active(app_data_dir: &Path, baseline_dir: &Path) -> Corpus {
    let source = load_catalog_source(app_data_dir).await;
    let dir = catalog_root(app_data_dir, &source);

    // Only the Bundled source seeds from the baseline (into app data). Managed /
    // UserClone roots are populated by provisioning (detect/clone/pull) — if one
    // is empty here it just hasn't been provisioned yet, so we serve what's
    // there (possibly empty) rather than stamping the baseline over a clone.
    if matches!(source, CatalogSource::Bundled) && is_empty_dir(&dir) {
        let seed_cats = discover_categories(baseline_dir);
        if let Err(e) = seed_from_baseline(baseline_dir, &dir, &seed_cats).await {
            tracing::warn!("corpus: seed from baseline failed: {e}");
        }
    }

    // Categories for indexing come from the ACTIVE root's tooling — after the
    // seed (or in a clone) `scripts/convert.sh` lives alongside the agents, so
    // the division set always reflects the catalog actually present.
    let categories = discover_categories(&dir);

    // Determine the version to stamp the index with: keep whatever a prior
    // refresh recorded, else the baseline marker.
    let version = match load_stored_meta(app_data_dir).await {
        Some(m) => m.version,
        None => BASELINE_VERSION.to_string(),
    };

    let mut corpus = match build_from_dir(&dir, &version, &categories).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("corpus: index build failed ({e}); serving empty corpus");
            empty_corpus(&version, &categories)
        }
    };

    // Prefer the catalog's own divisions.json (PR #592) for division label /
    // icon / color, falling back to the bundled metadata for first-run users
    // and pre-#592 clones that don't carry it yet.
    corpus.division_meta = categories::load_division_meta(&dir);

    // Persist index + meta (best effort — read commands work from the
    // in-memory copy regardless; the on-disk index exists for the
    // reconciliation subsystem built in a later phase).
    if let Err(e) = persist(app_data_dir, &corpus).await {
        tracing::warn!("corpus: persist index/meta failed: {e}");
    }

    corpus
}

/// Recursively collect every `*.md` under `root`, sorted by full path for
/// determinism. Real catalog clones nest agents in subdirectories (e.g.
/// `game-development/godot/<slug>.md`, `game-development/unity/<slug>.md`), so a
/// flat top-level scan would silently miss them.
fn collect_md_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            match ent.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(_) if path.extension().and_then(|e| e.to_str()) == Some("md") => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Find `<file_name>` anywhere under `dir` (depth-first). Used by `read_source`
/// to resolve a nested agent's canonical file when the flat path doesn't exist.
fn find_md_under(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(file_name) {
                return Some(path);
            }
        }
    }
    None
}

/// Build an in-memory [`Corpus`] by walking `<dir>/<category>/**/<slug>.md`
/// for every known category (recursively — real clones nest agents in
/// subdirs). Files without valid frontmatter (READMEs, workflow docs) are
/// skipped. The category is the top-level dir; the resulting `agents` vec and
/// `index` map are ordered deterministically by `(category, path)`.
pub(super) async fn build_from_dir(
    dir: &Path,
    version: &str,
    categories: &[String],
) -> Result<Corpus, AppError> {
    let mut rows: Vec<(Agent, CorpusEntry)> = Vec::new();

    for category in categories.iter() {
        let category = category.as_str();
        let cat_dir = dir.join(category);
        if !cat_dir.is_dir() {
            continue; // category dir absent — fine, skip.
        }
        // Recursive, sorted-by-path collection (catches nested agents).
        let files = collect_md_files(&cat_dir);

        for path in files {
            let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let raw = match read_capped(&path, MAX_AGENT_BYTES).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("corpus: skip {} ({e})", path.display());
                    continue;
                }
            };
            let source = match String::from_utf8(raw) {
                Ok(s) => s,
                Err(_) => {
                    tracing::warn!("corpus: skip {} (non-utf8)", path.display());
                    continue;
                }
            };
            match parse::parse_agent(slug, category, &source) {
                Ok(Some(pair)) => rows.push(pair),
                Ok(None) => {} // not an agent (no frontmatter) — skip silently.
                Err(e) => tracing::warn!("corpus: {e}"),
            }
        }
    }

    // `rows` is already in `(category, path)` order because we iterate
    // `categories` in tooling order and `collect_md_files` sorts by path.
    let mut agents = Vec::with_capacity(rows.len());
    let mut index = BTreeMap::new();
    for (agent, entry) in rows {
        index.insert(entry.slug.clone(), entry);
        agents.push(agent);
    }

    let count = index.len() as u32;
    Ok(Corpus {
        agents,
        index,
        category_order: categories.to_vec(),
        // Bundled floor; resolve_active overlays the catalog's divisions.json.
        division_meta: categories::bundled_division_meta(),
        meta: CorpusMeta {
            version: version.to_string(),
            commit: None,
            // The build itself carries no timestamp; fetched_at reflects
            // when the *content* was last fetched. For a baseline build
            // that is the seed time, captured at persist below if no meta
            // exists yet.
            fetched_at: String::new(),
            count,
        },
    })
}

fn empty_corpus(version: &str, categories: &[String]) -> Corpus {
    Corpus {
        agents: Vec::new(),
        index: BTreeMap::new(),
        category_order: categories.to_vec(),
        division_meta: categories::bundled_division_meta(),
        meta: CorpusMeta {
            version: version.to_string(),
            commit: None,
            fetched_at: String::new(),
            count: 0,
        },
    }
}

// ---------- Seeding ----------

/// True if `dir` does not exist or contains no entries.
pub fn is_empty_dir(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut it) => it.next().is_none(),
        Err(_) => true,
    }
}

/// Copy `<baseline>/<category>/*.md` into `<dest>/<category>/` for each
/// `category`, plus the repo tooling (`scripts/convert.sh`) so the seeded
/// working copy can discover its own divisions. Anything else in the baseline
/// is ignored. Idempotent: re-seeding overwrites file-for-file.
async fn seed_from_baseline(
    baseline: &Path,
    dest: &Path,
    categories: &[String],
) -> Result<(), AppError> {
    if !baseline.exists() {
        return Err(AppError::Io {
            message: format!("baseline corpus not found at {}", baseline.display()),
        });
    }
    let mut seeded = 0u32;
    for category in categories.iter() {
        let src_cat = baseline.join(category);
        let mut read = match tokio::fs::read_dir(&src_cat).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        let dst_cat = dest.join(category);
        tokio::fs::create_dir_all(&dst_cat)
            .await
            .map_err(|e| AppError::Io {
                message: format!("create {}: {e}", dst_cat.display()),
            })?;
        while let Ok(Some(ent)) = read.next_entry().await {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(fname) = path.file_name() else {
                continue;
            };
            let bytes = read_capped(&path, MAX_AGENT_BYTES).await?;
            atomic_write(&dst_cat.join(fname), &bytes).await?;
            seeded += 1;
        }
    }

    // Carry the tooling forward so the seeded copy is self-describing: the
    // category list is then read from the working tree, not just the baseline.
    let src_script = baseline.join("scripts").join("convert.sh");
    if let Ok(bytes) = read_capped(&src_script, MAX_AGENT_BYTES).await {
        let dst_script = dest.join("scripts").join("convert.sh");
        if let Some(parent) = dst_script.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = atomic_write(&dst_script, &bytes).await;
    }

    tracing::info!("corpus: seeded {seeded} agents from baseline");
    Ok(())
}

// ---------- Small fs helpers (used by read_source + tarball) ----------

/// Read up to `max` bytes; error (not truncate) on oversize. Mirrors
/// `util::fs::read_capped` but accepts a sync `Path` + tokio read so we
/// don't need to thread the catalog's exact helper here.
async fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, AppError> {
    let bytes = tokio::fs::read(path).await.map_err(|e| AppError::Io {
        message: format!("read {}: {e}", path.display()),
    })?;
    if bytes.len() as u64 > max {
        return Err(AppError::Io {
            message: format!("{} exceeds {} byte cap", path.display(), max),
        });
    }
    Ok(bytes)
}

/// Read the raw, byte-exact `.md` source of a seeded agent from the
/// active catalog root (`<app_data>/<catalog_root>/<category>/<slug>.md`).
/// Identity-tool installs (claude-code, copilot) ship this verbatim, and
/// provenance reconciliation re-renders against it. Path is derived from
/// app data + the agent's own category/slug — never from IPC input.
pub async fn read_source(
    app: &tauri::AppHandle,
    category: &str,
    slug: &str,
) -> Result<String, AppError> {
    let adir = super::app_data_dir(app)?;
    let source = load_catalog_source(&adir).await;
    let cat_dir = catalog_root(&adir, &source).join(category);
    let fname = format!("{slug}.md");
    // Flat path first (the common case); fall back to a recursive search
    // for nested agents (e.g. game-development/godot/<slug>.md in a real
    // clone).
    let flat = cat_dir.join(&fname);
    let path = if flat.exists() {
        flat
    } else {
        find_md_under(&cat_dir, &fname).unwrap_or(flat)
    };
    let bytes = read_capped(&path, MAX_AGENT_BYTES).await?;
    String::from_utf8(bytes).map_err(|e| AppError::Io {
        message: format!("agent source {slug}.md not UTF-8: {e}"),
    })
}

// ---------- Persistence ----------

/// Write `corpus-index.json` + `corpus-meta.json` atomically into the
/// state dir. The meta `fetched_at` is preserved from any prior meta;
/// when none exists (fresh baseline seed) it is stamped once with the
/// current UTC time so subsequent launches don't re-stamp it (keeps the
/// index byte-stable across launches).
async fn persist(app_data_dir: &Path, corpus: &Corpus) -> Result<(), AppError> {
    let sdir = paths::state_dir(app_data_dir);
    tokio::fs::create_dir_all(&sdir)
        .await
        .map_err(|e| AppError::Io {
            message: format!("create state dir {}: {e}", sdir.display()),
        })?;

    // Index — deterministic, no timestamp.
    let index_bytes = corpus.index_json()?;
    atomic_write(&paths::index_path(app_data_dir), &index_bytes).await?;

    // Meta — preserve prior fetched_at/commit if present; else stamp now.
    let prior = load_stored_meta(app_data_dir).await;
    let fetched_at = prior
        .as_ref()
        .map(|m| m.fetched_at.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let commit = prior.as_ref().and_then(|m| m.commit.clone());

    let stored = StoredMeta {
        version: corpus.meta.version.clone(),
        commit,
        fetched_at,
        count: corpus.count(),
    };
    let meta_bytes = serde_json::to_vec_pretty(&stored).map_err(|e| AppError::Internal {
        message: format!("serialize corpus-meta.json: {e}"),
    })?;
    atomic_write(&paths::meta_path(app_data_dir), &meta_bytes).await?;
    Ok(())
}

/// Load `corpus-meta.json` if present + parseable, else `None`.
async fn load_stored_meta(app_data_dir: &Path) -> Option<StoredMeta> {
    let path = paths::meta_path(app_data_dir);
    let bytes = tokio::fs::read(&path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ---------- Refresh (live tarball) ----------

/// Fetch the GitHub tarball, extract its category dirs over the working
/// copy, re-index, and persist. Returns the fresh [`CorpusMeta`].
///
/// The extraction is done into a temp dir first, then the known category
/// dirs are swapped in, so a partial/failed download never corrupts the
/// live `corpus/`.
pub async fn refresh(app_data_dir: &Path) -> Result<CorpusMeta, AppError> {
    // A read-only catalog source (Bundled-app-data is fine to refresh; a
    // user clone we lack permission to manage is NOT) must never be written by
    // a tarball refresh. Bundled writes into app data, so it's always allowed.
    let source = load_catalog_source(app_data_dir).await;
    if matches!(&source, CatalogSource::UserClone { manage: false, .. }) {
        return Err(AppError::InvalidArgument {
            message: "catalog source is a read-only user clone; enable manage-with-permission or switch source to refresh".into(),
        });
    }

    let bytes = download_corpus_tarball().await?;

    // Discover the live category set from the tarball's OWN tooling
    // (`scripts/convert.sh`) so a freshly-added upstream division is picked up
    // automatically. Falls back to the canonical default if absent.
    let categories =
        self::tarball::categories_from_tarball(&bytes).unwrap_or_else(bundled_division_slugs);

    // Extract the category dirs (+ the tooling) into the active catalog root.
    // The tarball has a single top-level `agency-agents-main/` prefix we strip.
    let dir = catalog_root(app_data_dir, &source);
    let extracted = self::tarball::extract_categories(&bytes, &dir, &categories)?;
    if extracted == 0 {
        return Err(AppError::Internal {
            message: "corpus tarball contained no agent files under known categories".into(),
        });
    }

    // Re-index from the freshly-written working copy. Use a `main`-tagged
    // version marker; codeload does not expose the resolved commit SHA in
    // the tarball, so we record the ref name. A later phase can resolve
    // the exact SHA via the GitHub API if needed.
    let version = format!("github:main@{}", chrono::Utc::now().format("%Y-%m-%d"));
    let mut corpus = build_from_dir(&dir, &version, &categories).await?;
    let fetched_at = chrono::Utc::now().to_rfc3339();
    corpus.meta.fetched_at = fetched_at.clone();

    // Persist a fresh meta (overwrite fetched_at/version this time —
    // unlike the baseline persist which preserves prior fetched_at).
    let sdir = paths::state_dir(app_data_dir);
    tokio::fs::create_dir_all(&sdir)
        .await
        .map_err(|e| AppError::Io {
            message: format!("create state dir {}: {e}", sdir.display()),
        })?;
    let index_bytes = corpus.index_json()?;
    atomic_write(&paths::index_path(app_data_dir), &index_bytes).await?;
    let stored = StoredMeta {
        version: version.clone(),
        commit: None,
        fetched_at: fetched_at.clone(),
        count: corpus.count(),
    };
    let meta_bytes = serde_json::to_vec_pretty(&stored).map_err(|e| AppError::Internal {
        message: format!("serialize corpus-meta.json: {e}"),
    })?;
    atomic_write(&paths::meta_path(app_data_dir), &meta_bytes).await?;

    Ok(corpus.meta)
}

/// Fetch the GitHub `codeload` tarball for the corpus (capped, timed out).
/// Shared by [`refresh`] and managed-catalog provisioning (the git-absent path).
pub async fn download_corpus_tarball() -> Result<Vec<u8>, AppError> {
    let client = reqwest::Client::builder()
        .timeout(REFRESH_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| AppError::Network {
            url: CORPUS_TARBALL_URL.to_string(),
            message: format!("client build: {e}"),
        })?;
    let resp = client
        .get(CORPUS_TARBALL_URL)
        .send()
        .await
        .map_err(|e| AppError::Network {
            url: CORPUS_TARBALL_URL.to_string(),
            message: e.to_string(),
        })?;
    if !resp.status().is_success() {
        return Err(AppError::HttpStatus {
            url: CORPUS_TARBALL_URL.to_string(),
            status: resp.status().as_u16(),
        });
    }
    let bytes = resp.bytes().await.map_err(|e| AppError::Network {
        url: CORPUS_TARBALL_URL.to_string(),
        message: format!("read body: {e}"),
    })?;
    if bytes.len() as u64 > MAX_TARBALL_BYTES {
        return Err(AppError::Io {
            message: format!(
                "corpus tarball {} bytes exceeds {} cap",
                bytes.len(),
                MAX_TARBALL_BYTES
            ),
        });
    }
    Ok(bytes.to_vec())
}
