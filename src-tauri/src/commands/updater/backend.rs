//! Updater backend abstraction — production plugin + test mock.
//!
//! Real plugin invocations require a `tauri::App` and a live network
//! call to the manifest endpoint, neither of which we want in unit
//! tests. The [`UpdaterBackend`] trait is the abstraction: production
//! uses [`PluginBackend`] (which calls the real plugin), tests use
//! [`MockBackend`] (which returns canned outcomes). Mirrors the
//! `KeychainSlot` pattern in `github::auth`.

use async_trait::async_trait;

use crate::error::AppError;

use super::types::CachedUpdate;

/// Trait-object façade around the `tauri-plugin-updater` so unit tests
/// can swap in a mock.
#[async_trait]
// The trait is referenced in test build via `MockBackend`; in test
// build the warn surfaces because `PluginBackend` (the only production
// implementer) is `#[cfg(not(test))]`. The lint is harmless — when
// the real test submodule is split into a sibling `tests` module it
// goes away.
#[allow(dead_code)]
pub trait UpdaterBackend: Send + Sync {
    /// Run a fresh manifest fetch. `Ok(None)` means "no update available",
    /// `Ok(Some(_))` means the plugin found a newer version. Errors are
    /// surfaced as `AppError` so the IPC contract stays uniform.
    async fn check(&self) -> Result<Option<CachedUpdate>, AppError>;

    /// Download + verify (sha256 + minisign) + install the update. The
    /// `version` arg is for sanity checking only; the plugin uses its
    /// own internal `Update` handle (cached from the most recent
    /// `check()` call) so we don't have to round-trip the full Update
    /// state through the trait boundary.
    async fn download_and_install(&self, version: &str) -> Result<(), AppError>;
}

// ===========================================================================
// Production backend
// ===========================================================================

/// Production backend that delegates to `tauri-plugin-updater`.
///
/// The plugin's `UpdaterExt::updater().check().await` returns a typed
/// `Update` value or `None`. We translate any failure into our typed
/// `AppError` family so the IPC contract stays uniform across the
/// surface.
#[cfg(not(test))]
pub struct PluginBackend<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

#[cfg(not(test))]
impl<R: tauri::Runtime> PluginBackend<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }

    /// Borrow the plugin's `Updater` value. Built fresh on each call —
    /// the plugin is cheap to instantiate and doing it eagerly at
    /// startup would force the manifest endpoint validation into the
    /// setup hook (failing builds with a malformed endpoint).
    fn updater(&self) -> Result<tauri_plugin_updater::Updater, AppError> {
        use tauri_plugin_updater::UpdaterExt;
        self.app.updater().map_err(|e| AppError::Internal {
            message: format!("updater plugin init: {e}"),
        })
    }
}

#[cfg(not(test))]
#[async_trait]
impl<R: tauri::Runtime> UpdaterBackend for PluginBackend<R> {
    async fn check(&self) -> Result<Option<CachedUpdate>, AppError> {
        let updater = self.updater()?;
        let opt = updater
            .check()
            .await
            .map_err(|e| translate_plugin_error(e, "update check"))?;
        let Some(update) = opt else {
            return Ok(None);
        };
        Ok(Some(CachedUpdate {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
            notes: update.body.clone(),
            pub_date: update.date.map(|d| d.to_string()),
        }))
    }

    async fn download_and_install(&self, version: &str) -> Result<(), AppError> {
        let updater = self.updater()?;
        let opt = updater
            .check()
            .await
            .map_err(|e| translate_plugin_error(e, "update check (pre-install)"))?;
        let Some(update) = opt else {
            // Manifest no longer advertises an update. The frontend
            // requested an install but the manifest changed underneath
            // us; surface as InvalidArgument so the UI can refresh.
            return Err(AppError::InvalidArgument {
                message: format!(
                    "manifest no longer advertises update for {version}; refresh and retry"
                ),
            });
        };

        // Re-check version match. The cached `update_install` arg
        // validator in `update_install` runs first against
        // `AppState.updater_state.cached_available`; this is a second
        // line of defense at the plugin boundary.
        if update.version != version {
            return Err(AppError::InvalidArgument {
                message: format!(
                    "manifest version drifted: requested {version}, manifest reports {}",
                    update.version
                ),
            });
        }

        // Download + verify (sha256 + minisign) → install. The plugin's
        // `download_and_install` runs both crypto checks; any failure
        // is translated to our typed `AppError` family.
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| translate_plugin_error(e, "update install"))?;

        Ok(())
    }
}

/// Map a plugin `Error` onto our typed `AppError` family. The plugin's
/// own error type carries enough context to discriminate hash mismatch
/// from signature failure, but its `Display` string is the most reliable
/// classifier across versions (the variants change between minor
/// releases).
#[cfg(not(test))]
fn translate_plugin_error(e: tauri_plugin_updater::Error, context: &str) -> AppError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    // Signature / minisign failures map to `SignatureVerificationFailed`
    // so the UI surfaces the same toast as a sha256 mismatch (both are
    // "the bytes did not verify, abort").
    if lower.contains("signature") || lower.contains("minisign") || lower.contains("pubkey") {
        AppError::SignatureVerificationFailed { message: msg }
    } else {
        AppError::Network {
            url: context.to_string(),
            message: msg,
        }
    }
}

// ===========================================================================
// Test mock
// ===========================================================================

/// Test double that returns canned outcomes. Each method has its own
/// pre-canned `Result` so tests can vary behaviour between check and
/// install without juggling state.
#[cfg(test)]
pub struct MockBackend {
    pub check: std::sync::Mutex<Option<Result<Option<CachedUpdate>, AppError>>>,
    pub install: std::sync::Mutex<Option<Result<(), AppError>>>,
}

#[cfg(test)]
impl MockBackend {
    /// Build a mock where `check()` returns `Ok(None)` (up to date) and
    /// `download_and_install()` returns `Ok(())`.
    #[allow(dead_code)] // future test surface
    pub fn returning(check: Result<Option<CachedUpdate>, AppError>) -> Self {
        Self {
            check: std::sync::Mutex::new(Some(check)),
            install: std::sync::Mutex::new(Some(Ok(()))),
        }
    }

    /// Build a mock that returns a fresh `Available` payload on `check()`.
    #[allow(dead_code)] // future test surface
    pub fn available(version: &str, current_version: &str) -> Self {
        Self::returning(Ok(Some(CachedUpdate {
            version: version.into(),
            current_version: current_version.into(),
            notes: None,
            pub_date: None,
        })))
    }

    /// Override the canned `install` outcome (default is `Ok(())`).
    #[allow(dead_code)] // future test surface
    pub fn install_returning(&self, r: Result<(), AppError>) {
        *self.install.lock().unwrap() = Some(r);
    }
}

#[cfg(test)]
#[async_trait]
impl UpdaterBackend for MockBackend {
    async fn check(&self) -> Result<Option<CachedUpdate>, AppError> {
        self.check
            .lock()
            .unwrap()
            .take()
            .expect("MockBackend::check called more than once")
    }

    async fn download_and_install(&self, _version: &str) -> Result<(), AppError> {
        self.install
            .lock()
            .unwrap()
            .take()
            .expect("MockBackend::download_and_install called more than once")
    }
}
