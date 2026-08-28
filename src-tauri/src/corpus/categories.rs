//! Bundled `divisions.json` overlay.
//!
//! Ships a snapshot of `agency-categories.json` in the app bundle; this is
//! the **floor** the app always has access to (used directly on first run /
//! for an old clone, and as the base that the active catalog's own
//! `divisions.json` overlays onto). See
//! [`super::mod`]'s docstring on the bundled-vs-active split for the
//! rationale.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;


const CATEGORIES_JSON: &str = include_str!("../../data/agency-categories.json");
pub(super) const DIVISIONS_FILENAME: &str = "divisions.json";

#[derive(Debug, Deserialize)]
struct CategoriesFile {
    categories: BTreeMap<String, CategoryMetaRow>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DivisionsFile {
    pub(super) divisions: BTreeMap<String, CategoryMetaRow>,
}

/// A single row from `agency-categories.json` / `divisions.json`: the
/// presentation metadata for a division. `color` defaults to the neutral
/// slate so old catalogs without a `color` field still parse cleanly.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CategoryMetaRow {
    pub(super) label: String,
    pub(super) icon: String,
    #[serde(default = "default_division_color")]
    pub(super) color: String,
}

/// Neutral fallback color for a division without one in the metadata.
pub(super) fn default_division_color() -> String {
    "#94A3B8".to_string()
}

/// The bundled `agency-categories.json` parsed into a slug -> row map. This is
/// the floor the app always ships — used directly on first run / for an old
/// clone, and as the base that `divisions.json` overlays onto.
pub(super) fn bundled_division_meta() -> BTreeMap<String, CategoryMetaRow> {
    serde_json::from_str::<CategoriesFile>(CATEGORIES_JSON)
        .map(|f| f.categories)
        .unwrap_or_default()
}

/// The bundled division slugs (offline default) — the keys of the bundled floor,
/// sorted. Used where the active catalog's own `divisions.json` isn't available
/// to enumerate divisions from (e.g. a tarball with no metadata, or detection).
pub(super) fn bundled_division_slugs() -> Vec<String> {
    let mut v: Vec<String> = bundled_division_meta().into_keys().collect();
    v.sort();
    v
}

/// Resolve division metadata for the active catalog: start from the bundled
/// floor, then overlay the catalog root's `divisions.json` (PR #592 — the
/// canonical source shared with the CLI installer + linters) when present and
/// parseable. First-run (Bundled) users and pre-#592 clones simply have no
/// `divisions.json`, so they keep the bundled metadata — no drift, no failure.
/// Overlaying (rather than replacing) means a `divisions.json` that omits a
/// division still falls back to the bundled row for it.
pub(super) fn load_division_meta(
    catalog_root: &Path,
) -> BTreeMap<String, CategoryMetaRow> {
    let mut meta = bundled_division_meta();
    let path = catalog_root.join(DIVISIONS_FILENAME);
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<DivisionsFile>(&raw) {
            Ok(file) => {
                for (slug, row) in file.divisions {
                    meta.insert(slug, row);
                }
                tracing::debug!("corpus: division metadata sourced from {}", path.display());
            }
            Err(e) => tracing::warn!(
                "corpus: {} present but unparseable ({e}); using bundled division metadata",
                path.display()
            ),
        },
        // Absent is the common, expected case (first run / old clone) — not a warning.
        Err(_) => tracing::debug!(
            "corpus: no {DIVISIONS_FILENAME} at catalog root; using bundled division metadata"
        ),
    }
    meta
}

/// Resolve `(label, icon, color)` for a category slug from a resolved division
/// metadata map. Falls back to a title-cased slug + a neutral `Folder` icon +
/// a neutral color if the slug is somehow absent (keeps Discover rendering
/// rather than dropping a tile).
pub(super) fn category_meta_from(
    meta: &BTreeMap<String, CategoryMetaRow>,
    slug: &str,
) -> (String, String, String) {
    match meta.get(slug) {
        Some(row) => (row.label.clone(), row.icon.clone(), row.color.clone()),
        None => (
            title_case(slug),
            "Folder".to_string(),
            default_division_color(),
        ),
    }
}

/// `"game-development"` —> `"Game Development"`. Deterministic fallback for
/// the unlikely missing-slug case.
pub(super) fn title_case(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
