//! Integration test that drives the ts-rs code generator for every
//! public DTO in `src/types.rs`.
//!
//! ts-rs is configured to write its output to
//! `src/lib/types.generated.ts`. Each `cargo test` invocation
//! regenerates the file — it's deterministic and idempotent.
//! The CI drift check (see the [ci] workflow note in
//! `.github/workflows/ci.yml`) re-runs this test and fails the build
//! if the regenerated file differs from what's committed.
//!
//! The `import_types!()` macro triggers generation — it isn't a
//! test that runs anything at runtime; the `#[test]` function just
//! exists so `cargo test --test ts_export` is the entry point the
//! CI drift check calls.
use ts_rs::TS;

use rubezhanin_agency_agents_lib::types::{
    Agent, AgentDiff, BackupEntry, CatalogCandidate, CatalogDetection,
    CatalogSource, CatalogStatus, CatalogUpdateCheck, Category, CorpusEntry,
    CorpusMeta, InstallRecord, InstallState, InstalledAgent, LogFile,
    ProjectInfo, Scope, ToolInfo, ToolVersion, UpdateKind,
};
// `install_kind` and `slug_from` are raw `String` fields on
// `ToolEntry` (validated post-parse so the bundled `tools.json`
// doesn't have to be rewritten for Phase 2). ts-rs exports
// those via the `ToolEntry` type itself; we import the whole
// type rather than the per-field string aliases.
use rubezhanin_agency_agents_lib::CrateRootToolEntry as ToolEntry;
use rubezhanin_agency_agents_lib::CrateRootToolManifest as ToolManifest;
// Recovery types live behind the private `install` module. The
// `lib.rs`-root `pub use` re-exports them under the alias names
// `CrateRoot*`; we import those here, then `export_all` on them
// drives the ts-rs codegen for the real types in `recovery.rs`.
use rubezhanin_agency_agents_lib::{
    CrateRootRecoveryAction as RecoveryAction,
    CrateRootRecoveryReport as RecoveryReport,
};

#[test]
fn regenerate_typescript_bindings() {
    Agent::export_all().expect("export Agent");
    AgentDiff::export_all().expect("export AgentDiff");
    BackupEntry::export_all().expect("export BackupEntry");
    CatalogCandidate::export_all().expect("export CatalogCandidate");
    CatalogDetection::export_all().expect("export CatalogDetection");
    CorpusMeta::export_all().expect("export CorpusMeta");
    CatalogSource::export_all().expect("export CatalogSource");
    CatalogStatus::export_all().expect("export CatalogStatus");
    CatalogUpdateCheck::export_all().expect("export CatalogUpdateCheck");
    Category::export_all().expect("export Category");
    CorpusEntry::export_all().expect("export CorpusEntry");
    InstallRecord::export_all().expect("export InstallRecord");
    InstallState::export_all().expect("export InstallState");
    InstalledAgent::export_all().expect("export InstalledAgent");
    LogFile::export_all().expect("export LogFile");
    ProjectInfo::export_all().expect("export ProjectInfo");
    RecoveryAction::export_all().expect("export RecoveryAction");
    RecoveryReport::export_all().expect("export RecoveryReport");
    Scope::export_all().expect("export Scope");
    ToolEntry::export_all().expect("export ToolEntry");
    ToolInfo::export_all().expect("export ToolInfo");
    ToolManifest::export_all().expect("export ToolManifest");
    ToolVersion::export_all().expect("export ToolVersion");
    UpdateKind::export_all().expect("export UpdateKind");
}
