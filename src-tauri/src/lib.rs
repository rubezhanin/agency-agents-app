//! Agency Agents — Tauri 2 backend entrypoint. Maintained by Yuri Shvets
//! (https://github.com/rubezhanin).
//!
//! Module layout per `memory-bank/backendApi.md` §9. This file is the
//! Tauri Builder + invoke_handler registration; every command lives
//! in `commands::*`.

mod audit;
mod commands;
mod corpus;
mod error;
mod github;
mod hermes;
mod install;
mod manifest;
mod registry;
mod render;
mod state;
// `types` is intentionally `pub` (not `pub(crate)`) so the ts-rs
// integration test in `tests/ts_export.rs` can `use
// rubezhanin_agency_agents_lib::types::*` to drive the codegen. The
// crate is a Tauri lib (staticlib / cdylib / rlib), so the only
// "consumer" of this surface is the Tauri build itself and the
// TypeScript frontend \u2014 making the types module `pub` doesn't widen
// any user-facing API beyond what's already implicit in a Tauri app.
pub mod types;
mod util;

// Re-export the transactional-engine recovery types at the
// crate root so the ts-rs integration test in `tests/ts_export.rs`
// can drive their codegen without going through the private
// `install` module.
pub use crate::install::recovery::{
    RecoveryAction as CrateRootRecoveryAction, RecoveryReport as CrateRootRecoveryReport,
};
// Re-export the upstream tool manifest types at the crate
// root for the same reason. Phase 3 (plugin architecture) will
// fold `manifest` and `registry` into one surface and these
// crate-root aliases will go away.
pub use crate::manifest::{ToolEntry as CrateRootToolEntry, ToolManifest as CrateRootToolManifest};
// Re-export the plan / dry-run types for the same reason.
pub use crate::commands::plan::{
    DeployPlan as CrateRootDeployPlan, PlanChange as CrateRootPlanChange,
    PlanSummary as CrateRootPlanSummary,
};
// Re-export the Hermes pre-flight types for the same reason.
pub use crate::hermes::{
    HermesPreflight as CrateRootHermesPreflight, PreflightCheck as CrateRootPreflightCheck,
    PreflightStatus as CrateRootPreflightStatus,
};
// Re-export the Hermes probe + probe source for the same reason
// (Phase 4c — HermesHealthSnapshot embeds both).
pub use crate::hermes::probe::{HermesProbe as CrateRootHermesProbe, ProbeSource as CrateRootProbeSource};
// Re-export the audit log DTOs (Phase 5 — Trustworthy Core: runbook
// apply + audit trail).
pub use crate::audit::{AuditEntry as CrateRootAuditEntry, AuditOutcome as CrateRootAuditOutcome};
// Re-export the Hermes installed-plugin DTO (Phase 4b — multi-plugin
// routing). Lives in `commands::hermes` so the ts-rs integration test
// reaches it via the same crate-root alias pattern.
pub use crate::commands::hermes::{
    HermesHealthSnapshot as CrateRootHermesHealthSnapshot,
    HermesHealthStatus as CrateRootHermesHealthStatus,
    HermesInstalledPlugin as CrateRootHermesInstalledPlugin,
};

use commands::*;

// =============================================================
// Phase 15 — Updater minisign public key
// =============================================================
//
// The public key half of the minisign keypair used to sign release
// .dmg artifacts. Public keys are public by design — embedding them
// in the binary is the standard pattern for offline-verified updates
// (Sparkle, Tauri, every shipping Mac auto-updater).
//
// **Placeholder.** Replace before cutting a release. The real key is
// generated per `BUILD.md` instructions:
//
//     tauri signer generate -w ~/.config/agency-agents-app/updater.key
//
// The matching public key the command prints is what goes here.
// Keep the private key chmod 600 outside the repo — it's the only
// thing standing between a compromised agency-agents-app.rubezhanin.app and a
// malicious binary push.
//
// Real minisign public key. The matching private key lives at
// `~/.config/agency-agents-app/updater.key` (chmod 600,
// outside the repo). The signature verification at install time
// validates every downloaded `.app.tar.gz` against this pubkey; any
// mismatch aborts the install with no on-disk side effects.
//
// `tauri.conf.json` carries the same value for the plugin to consume
// at startup; keep both in sync. The plugin parses Tauri's base64-of-
// minisign-blob format directly — what you see here is exactly what
// `tauri signer generate -w …` printed.
const UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEFCRjVBRkQ4ODhFRDI5QkQKUldTOUtlMkkySy8xcTlyRnNjM1pkMy9sc2NkMzdOOVlhTEd5OTVoNFIwWDI4VUROUGhVbFNuMzMK";

pub fn updater_pubkey() -> &'static str {
    UPDATER_PUBKEY
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMABUF renderer aborts with "Could not create default EGL
    // display: EGL_BAD_PARAMETER" on a lot of Linux GPU/driver stacks (Arch,
    // NVIDIA, Wayland, newer Mesa) — the webview never comes up (issue #641).
    // Forcing the non-DMABUF path before GTK/WebKit initializes fixes it, at a
    // negligible rendering cost. Only touch it when the user hasn't set it
    // themselves, so an explicit override still wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // Best-effort tracing setup is deferred to `setup()` so we know
    // the app-data directory before we create the rolling file
    // appender. The Tauri path resolver (`app.path().app_data_dir()`)
    // is the only authoritative way to find that directory on
    // every platform (macOS / Linux / Windows each have their own
    // conventions and `BundleIdentifier` rules).
    //
    // Until `setup()` runs, log events from this process are simply
    // dropped — that's a few microseconds of `tracing::info!` from
    // Tauri's own bootstrap. Nothing user-facing happens in that
    // window.

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Phase 15 — register the updater plugin. The endpoint URL and
        // public key are configured in `tauri.conf.json`; the plugin
        // pulls them from the parsed Config at startup. Our IPC
        // wrappers in `commands::updater` route every check + install
        // through `state.require_network("update_check")` first so
        // Offline Mode kills the path even though the plugin itself
        // would otherwise try the manifest endpoint.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Issue #17 — persist the window's size + position across launches.
        // The plugin auto-saves geometry when the window is moved/resized and
        // on exit, then restores it on the next launch. Default StateFlags
        // cover size + position (plus maximized/fullscreen) — exactly what the
        // issue asks for. No frontend wiring: registration is the feature.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .menu(build_app_menu)
        .on_menu_event(handle_menu_event)
        // Persist window geometry on every resize/move, not just on graceful
        // exit — so a size change survives even a hard kill (e.g. stopping
        // `tauri dev` with Ctrl-C, which never runs the exit-save handler).
        // The state file is tiny; the OS coalesces the writes during a drag.
        .on_window_event(|window, event| {
            use tauri::Manager;
            use tauri_plugin_window_state::{AppHandleExt, StateFlags};
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_)
            ) {
                let _ = window.app_handle().save_window_state(StateFlags::all());
            }
        })
        .setup(|app| {
            state::initialize(app)?;

            // 0.4.7 — Initialise the tracing subscriber now that we
            // have the app-data directory. Two layers: the original
            // stderr layer (handy for `tauri dev`) and a daily-
            // rolling JSON file under `app_data/logs/app.YYYY-MM-DD`.
            // The frontend reads those files via the `logs_*` IPC
            // commands (Settings → Logs).
            use tauri::Manager;
            use tracing_appender::rolling::{RollingFileAppender, Rotation};
            use tracing_subscriber::{
                fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
            };
            let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new("warn,rubezhanin_agency_agents_app=info")
            });
            // Best-effort — if the app-data dir isn't writable (e.g.
            // running off a read-only volume in a CI harness), the
            // stderr layer still works, the file layer just goes
            // missing silently.
            let logs_dir = app
                .path()
                .app_data_dir()
                .map(|d| d.join("logs"))
                .ok();
            let file_layer = logs_dir.and_then(|dir| {
                std::fs::create_dir_all(&dir).ok()?;
                let appender = RollingFileAppender::new(Rotation::DAILY, dir, "app.json");
                Some(
                    fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(false)
                        .with_writer(appender)
                        .with_filter(env_filter.clone()),
                )
            });
            let stderr_layer = fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(env_filter);
            let _ = tracing_subscriber::registry()
                .with(stderr_layer)
                .with(file_layer)
                .try_init();
            // Phase 15 — spawn the auto-check scheduler. The task
            // sleeps for 24h between wakes, re-reads the live settings
            // on each cycle (so a user toggling auto-check off mid-run
            // is honoured on the next wake), and runs the check only
            // when both `update_auto_check` is on AND `paranoid_mode`
            // is off. Backoff on failure: 1h → 6h → 24h.
            commands::updater::spawn_auto_check_scheduler(app.handle().clone());

            // 0.4.7-dev — startup recovery for the operation journal.
            // If a previous run died mid-install (Ctrl-C, OOM,
            // BSOD), the journal will have `pending` / `committing`
            // rows that never reached a terminal state. Sweep
            // them, mark them as `failed`, and surface a one-line
            // warning per affected op. The user-facing banner
            // (showing the affected dests) is delivered via the
            // `journal_recovery` event in a follow-up commit; for
            // now we just log.
            if let Ok(adir) = app.path().app_data_dir() {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use crate::install::recovery;
                    use tauri::Emitter;
                    match recovery::recover_unfinished(&adir).await {
                        Ok(report) if report.recovered_count > 0 => {
                            tracing::warn!(
                                recovered_count = report.recovered_count,
                                found_count = report.found_count,
                                actions = report.actions.len(),
                                "startup: recovered unfinished operations from the previous run; \
                                 affected dests need a manual reconcile or backup_restore"
                            );
                            // Emit a Tauri event so the frontend
                            // (when wired) can show a banner. We
                            // ship the event now so the IPC plumbing
                            // is in place even before the UI listens
                            // to it; an event with no listeners is
                            // a no-op, not an error.
                            let _ = app_handle.emit(
                                "journal_recovery",
                                report,
                            );
                        }
                        Ok(_) => {
                            // No recovery needed; journal is clean.
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "startup: journal recovery failed; ignoring (recovery is \
                                 best-effort — a corrupt journal should not block app startup)"
                            );
                        }
                    }
                });
            }
            #[cfg(target_os = "macos")]
            {
                // Apply NSVisualEffectView to the main window so it picks up the
                // native macOS "frosted glass" appearance. Material::HudWindow
                // gives a slightly heavier blur that looks good behind the
                // sidebar and main panes; the WebView background must be set
                // transparent in CSS (see app.css :root) for the blur to show.
                use tauri::Manager;
                use window_vibrancy::{
                    apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                };
                if let Some(window) = app.get_webview_window("main") {
                    let _ = apply_vibrancy(
                        &window,
                        NSVisualEffectMaterial::HudWindow,
                        Some(NSVisualEffectState::Active),
                        None,
                    );
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            settings_get,
            settings_set,
            settings_reset,
            github_repo_stats,
            github_status,
            github_signin_start,
            github_signin_poll,
            github_signout,
            github_star,
            github_unstar,
            github_is_starred,
            github_watch,
            github_unwatch,
            github_create_issue,
            update_check_now,
            update_install,
            update_skip,
            update_relaunch,
            // Hermes plugin installer + CLI detection. The plugin format is
            // documented in `docs/HERMES-PLUGIN.md`; the CLI detection is a
            // port of `rubezhanin/agent-kit` `src/hermes/{probe,scan}.ts`.
            commands::hermes::hermes_status,
            commands::hermes::hermes_install,
            commands::hermes::hermes_uninstall,
            commands::hermes::hermes_stage,
            // Phase 4a — readiness check (CLI / kanban / Node / home /
            // install target). Pure read; doesn't block the install
            // buttons but surfaces a colour-coded checklist in the
            // Settings → Hermes tile.
            commands::hermes::hermes_preflight,
            // Phase 4b — multi-plugin routing: scan
            // `~/.hermes/plugins/` and return a per-plugin summary
            // (id, label, path, agent count, canonical flag).
            commands::hermes::hermes_list_plugins,
            // Phase 4c — aggregated health snapshot (probe + preflight +
            // installed plugins in a single round-trip). The frontend
            // polls this on a 60s timer while the Hermes settings
            // section is mounted.
            commands::hermes::hermes_health,
            // Phase 1 — corpus subsystem (contracts.md §C). These live in
            // the `corpus` module rather than `commands::*`; register them
            // fully-qualified alongside the other commands.
            corpus::corpus_status,
            corpus::corpus_refresh,
            corpus::corpus_list,
            corpus::corpus_get,
            corpus::corpus_categories,
            corpus::catalog_source_get,
            corpus::catalog_configured,
            corpus::catalog_source_set,
            corpus::catalog_detect,
            corpus::catalog_provision_managed,
            corpus::catalog_pull,
            corpus::catalog_status,
            corpus::catalog_check_updates,
            corpus::runbooks::runbooks_list,
            // Phase 2 — install + reconcile (contracts.md §C). The cross-tool
            // agent state layer: render/ledger/reconcile/tools/projects.
            install::install_agent,
            install::update_agent,
            install::track_agent,
            install::agent_diff,
            install::uninstall_agent,
            install::project_forget,
            install::installs_reconcile,
            install::installs_for_agent,
            install::tools_list,
            install::tool_versions,
            install::reveal_path,
            install::projects_list,
            install::loadout_export,
            install::loadout_import,
            // Phase 2b — backup / rollback UI (app_data/backups/ + index.json).
            install::backup_list,
            install::backup_restore,
            install::backup_folder_path,
            // Phase 3 — Plan / Dry Run: pure-function preview
            // of an install's filesystem effects. Surface in
            // the UI as a pre-flight modal.
            commands::plan::deploy_plan,
            // Phase 0.4.7 — structured logs (app_data/logs/app.YYYY-MM-DD.json).
            commands::logs_list,
            commands::logs_read,
            commands::logs_clear,
            commands::logs_folder_path,
            // Phase 5 — audit log (app_data/audit/operations.jsonl).
            // Backs the Settings → Activity tab with a durable,
            // crash-safe trail of significant operations.
            commands::audit_log,
            commands::audit_recent,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// =============================================================
// Native macOS menu (Phase 12+)
// =============================================================
//
// macOS apps have a system menu bar above the screen, not inside the window.
// The "App" menu is the first item (named after the app) and is where users
// expect to find "About <App>" and "Settings…". Per Tauri 2 conventions we
// build the menu in a closure passed to `.menu(...)` on the Builder, and
// dispatch click events from `.on_menu_event(...)`.
//
// The menu items emit Tauri events that the frontend listens for via
// `listen()` and turns into store-state updates (`ui.openAbout()` /
// `ui.openSettings()`). This keeps the menu definition Rust-side and the
// modal rendering entirely in Svelte.

const MENU_EVENT_ABOUT: &str = "agency-agents/menu/about";
const MENU_EVENT_SETTINGS: &str = "agency-agents/menu/settings";

fn build_app_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};

    let pkg = app.package_info();

    // App menu: About (custom — opens our in-app modal), Settings…, ─, Hide
    // / Hide-Others / Show-All, ─, Quit. The native PredefinedMenuItem::about
    // would open the OS dialog; we route through our own modal instead via
    // a MenuItemBuilder + the menu event so the donate CTA + Anthropic
    // credits render in our UI.
    let about_item = MenuItemBuilder::new(format!("About {}", pkg.name))
        .id(MENU_EVENT_ABOUT)
        .build(app)?;
    let settings_item = MenuItemBuilder::new("Settings…")
        .id(MENU_EVENT_SETTINGS)
        .accelerator("CmdOrCtrl+,")
        .build(app)?;

    let app_submenu = SubmenuBuilder::new(app, pkg.name.clone())
        .item(&about_item)
        .separator()
        .item(&settings_item)
        .separator()
        .item(&PredefinedMenuItem::hide(app, None)?)
        .item(&PredefinedMenuItem::hide_others(app, None)?)
        .item(&PredefinedMenuItem::show_all(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, None)?)
        .build()?;

    // Standard ancillary menus — Edit (copy/paste/etc.) + Window. Pure
    // PredefinedMenuItems so we don't have to reinvent them.
    let edit_submenu = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    let window_submenu = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, None)?)
        .build()?;

    MenuBuilder::new(app)
        .item(&app_submenu)
        .item(&edit_submenu)
        .item(&window_submenu)
        .build()
}

fn handle_menu_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: tauri::menu::MenuEvent) {
    use tauri::Emitter;
    match event.id().as_ref() {
        MENU_EVENT_ABOUT => {
            let _ = app.emit("menu:about", ());
        }
        MENU_EVENT_SETTINGS => {
            let _ = app.emit("menu:settings", ());
        }
        _ => {}
    }
}
