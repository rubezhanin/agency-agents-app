//! Integration tests for the settings submodule tree.
//!
//! Verifies the load → persist → reload round-trip, the numeric
//! clamps on save and load, the wire-shape (camelCase JSON keys), the
//! fail-closed behaviour on corrupt / oversize / missing files, and
//! the skip-list cap-and-dedupe helper.

mod tests {
    use crate::commands::settings::{
        load_async, load_at_startup, persist, settings_path, update, CaskIconMode, Settings,
        SettingsLoadState, MAX_SETTINGS_BYTES,
    };
    use std::collections::HashMap;

    /// File-absent → defaults apply (paranoid OFF).
    #[tokio::test]
    async fn missing_file_is_first_launch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::FirstLaunch => {}
            other => panic!("expected FirstLaunch, got {other:?}"),
        }
        // Defaults must have paranoid OFF.
        let effective = state
            .effective_settings()
            .expect("first launch has defaults");
        assert!(!effective.paranoid_mode);
    }

    /// File-corrupt (bad JSON) → fail closed.
    #[tokio::test]
    async fn corrupt_file_fails_closed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        tokio::fs::write(&path, b"{not valid json").await.unwrap();

        let state = load_at_startup(tmp.path());
        match &state {
            SettingsLoadState::Corrupt { message } => {
                assert!(message.contains("parse"), "{message}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
        // effective_settings must be None — caller must NOT see "paranoid off".
        assert!(state.effective_settings().is_none());
    }

    /// File-oversize → fail closed.
    #[tokio::test]
    async fn oversize_file_fails_closed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Write 1 MiB + 1 byte.
        let payload = vec![b'a'; (MAX_SETTINGS_BYTES + 1) as usize];
        tokio::fs::write(&path, &payload).await.unwrap();

        let state = load_at_startup(tmp.path());
        match &state {
            SettingsLoadState::Corrupt { message } => {
                assert!(message.contains("exceeds"), "{message}");
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    /// Round-trip: persist + reload returns the same struct.
    #[tokio::test]
    async fn round_trip_persists_all_fields() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            paranoid_mode: true,
            catalog_stale_banner_days: 21,
            cask_icon_mode: CaskIconMode::InstalledOnly,
            trending_ttl_minutes: 120,
            github_enabled: true,
            ai_features_enabled: false,
            update_auto_check: true,
            skipped_update_versions: vec!["0.3.0".into(), "0.3.1".into()],
            enhanced_trending_enabled: true,
            vulnerability_scanning_enabled: true,
            live_enrichment_enabled: true,
            tool_paths: HashMap::from([("claudeCode".to_string(), "/wsl/home/me".to_string())]),
        };
        let written = persist(tmp.path(), s.clone()).await.expect("persist");
        assert_eq!(written, s);

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert_eq!(loaded, s),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Phase 12c — `github_enabled` must round-trip with the camelCase
    /// JSON key `githubEnabled`. The field is brand-new and we want a
    /// pinning test that the wire shape matches the frontend type.
    #[tokio::test]
    async fn github_enabled_round_trips_with_camel_case_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            github_enabled: true,
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        // Inspect raw JSON on disk for the camelCase key. We don't want a
        // future serde rename to silently shift the wire shape.
        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"githubEnabled\""),
            "expected camelCase key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"github_enabled\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert!(loaded.github_enabled),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Out-of-range numerics get clamped on save.
    #[tokio::test]
    async fn clamps_on_save() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            paranoid_mode: false,
            catalog_stale_banner_days: 9999, // way above 365
            cask_icon_mode: CaskIconMode::All,
            trending_ttl_minutes: 1, // below the 5-minute floor
            github_enabled: false,
            ai_features_enabled: true,
            update_auto_check: false,
            skipped_update_versions: Vec::new(),
            enhanced_trending_enabled: false,
            vulnerability_scanning_enabled: false,
            live_enrichment_enabled: false,
            tool_paths: HashMap::new(),
        };
        let written = persist(tmp.path(), s).await.expect("persist");
        assert_eq!(
            written.catalog_stale_banner_days,
            Settings::CATALOG_STALE_DAYS_MAX
        );
        assert_eq!(written.trending_ttl_minutes, Settings::TRENDING_TTL_MIN);
    }

    /// Out-of-range numerics get clamped on read too (defense against
    /// hand-edited settings.json).
    #[tokio::test]
    async fn clamps_on_load() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Hand-write a settings file with absurd values.
        let raw = br#"{
            "paranoidMode": false,
            "catalogStaleBannerDays": 99999,
            "caskIconMode": "all",
            "trendingTtlMinutes": 2
        }"#;
        tokio::fs::write(&path, raw).await.unwrap();

        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => {
                assert_eq!(
                    s.catalog_stale_banner_days,
                    Settings::CATALOG_STALE_DAYS_MAX
                );
                assert_eq!(s.trending_ttl_minutes, Settings::TRENDING_TTL_MIN);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Unknown enum variant → serde rejects the parse → fail closed
    /// (intentional; we don't want a typo'd field to silently pick a
    /// default the user didn't write).
    ///
    /// The plan asks for "default substituted", but serde's parser is
    /// all-or-nothing on a single field — we can't selectively recover
    /// one unknown variant while keeping the rest. The fail-closed
    /// behaviour is the strictly safer interpretation: the user's
    /// "deny network until repaired" gate kicks in, the UI surfaces
    /// the parse error, and the user hits Reset to defaults. The doc
    /// comment on `SettingsLoadState::Corrupt` explains this.
    #[tokio::test]
    async fn unknown_enum_variant_is_corrupt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        let raw = br#"{
            "paranoidMode": false,
            "catalogStaleBannerDays": 14,
            "caskIconMode": "every-blue-moon",
            "trendingTtlMinutes": 60
        }"#;
        tokio::fs::write(&path, raw).await.unwrap();

        let state = load_at_startup(tmp.path());
        match &state {
            SettingsLoadState::Corrupt { message } => {
                assert!(
                    message.contains("parse"),
                    "expected parse failure in corrupt message, got {message}"
                );
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
        assert!(state.effective_settings().is_none(), "must fail closed");
    }

    /// Missing optional fields take their defaults (forward compat).
    #[tokio::test]
    async fn missing_fields_use_defaults() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Only paranoidMode set — everything else absent.
        let raw = br#"{ "paranoidMode": true }"#;
        tokio::fs::write(&path, raw).await.unwrap();

        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => {
                assert!(s.paranoid_mode);
                assert_eq!(s.catalog_stale_banner_days, 14);
                assert_eq!(s.cask_icon_mode, CaskIconMode::All);
                assert_eq!(s.trending_ttl_minutes, 60);
                // `github_enabled` was added in 12c — must default to false
                // for forward compat with pre-12c settings files.
                assert!(!s.github_enabled);
                // `ai_features_enabled` was added in Phase 13 — must
                // default to true for forward compat with pre-13 settings
                // files (pre-existing installs see categories + enrichment
                // turned on as soon as they upgrade).
                assert!(s.ai_features_enabled);
                // `update_auto_check` was added in Phase 15 — must default
                // to false for forward compat with pre-15 settings files.
                assert!(!s.update_auto_check);
                // `skipped_update_versions` was added in Phase 15 — must
                // default to an empty vec.
                assert!(s.skipped_update_versions.is_empty());
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    // ---------- Phase 15 — skip-list cap + helpers ----------

    /// Push helper adds entries in order until the cap is reached.
    #[test]
    fn push_skipped_version_appends_until_cap() {
        let mut s = Settings::default();
        for i in 0..Settings::SKIPPED_UPDATE_VERSIONS_CAP {
            let changed = s.push_skipped_version(format!("0.3.{i}"));
            assert!(changed, "first-time push of unique version must change");
        }
        assert_eq!(
            s.skipped_update_versions.len(),
            Settings::SKIPPED_UPDATE_VERSIONS_CAP
        );
    }

    /// Phase 15 §Tests #5 — adding the 11th skip evicts the oldest entry.
    /// This is the canonical bound test.
    #[test]
    fn push_skipped_version_evicts_oldest_on_overflow() {
        let mut s = Settings::default();
        // Fill to cap.
        for i in 0..Settings::SKIPPED_UPDATE_VERSIONS_CAP {
            s.push_skipped_version(format!("v{i}"));
        }
        assert_eq!(s.skipped_update_versions[0], "v0");

        // 11th push: oldest (v0) must be gone, newest (vN) must be at tail.
        let new_version = format!("v{}", Settings::SKIPPED_UPDATE_VERSIONS_CAP);
        s.push_skipped_version(new_version.clone());
        assert_eq!(
            s.skipped_update_versions.len(),
            Settings::SKIPPED_UPDATE_VERSIONS_CAP
        );
        assert!(
            !s.skipped_update_versions.contains(&"v0".to_string()),
            "oldest entry v0 should have been evicted"
        );
        assert_eq!(
            s.skipped_update_versions.last(),
            Some(&new_version),
            "newest entry should be at tail"
        );
    }

    /// Re-pushing an existing version moves it to the tail without
    /// growing the list past the cap.
    #[test]
    fn push_skipped_version_dedupes_and_moves_to_tail() {
        let mut s = Settings::default();
        s.push_skipped_version("a".into());
        s.push_skipped_version("b".into());
        s.push_skipped_version("c".into());

        // Re-push "a" — should move to tail, length unchanged.
        let changed = s.push_skipped_version("a".into());
        assert!(changed);
        assert_eq!(s.skipped_update_versions, vec!["b", "c", "a"]);

        // Pushing the current tail again is a no-op.
        let changed = s.push_skipped_version("a".into());
        assert!(!changed);
        assert_eq!(s.skipped_update_versions, vec!["b", "c", "a"]);
    }

    /// Hand-edited settings.json with a too-long skip list gets pruned
    /// on load via clamp().
    #[test]
    fn clamp_prunes_oversized_skip_list() {
        let mut s = Settings::default();
        for i in 0..(Settings::SKIPPED_UPDATE_VERSIONS_CAP * 3) {
            s.skipped_update_versions.push(format!("v{i}"));
        }
        s.clamp();
        assert_eq!(
            s.skipped_update_versions.len(),
            Settings::SKIPPED_UPDATE_VERSIONS_CAP
        );
        // The most-recent half is retained; the oldest two-thirds are dropped.
        assert!(
            !s.skipped_update_versions.contains(&"v0".to_string()),
            "oldest entries should have been dropped"
        );
    }

    /// Phase 15 — wire shape gate. The new fields must round-trip with
    /// camelCase JSON keys (`updateAutoCheck`, `skippedUpdateVersions`)
    /// so the frontend store can rely on the contract.
    #[tokio::test]
    async fn phase15_fields_round_trip_with_camel_case_keys() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            update_auto_check: true,
            skipped_update_versions: vec!["1.0.0".into()],
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"updateAutoCheck\""),
            "expected camelCase updateAutoCheck key in raw JSON, got: {raw}"
        );
        assert!(
            raw.contains("\"skippedUpdateVersions\""),
            "expected camelCase skippedUpdateVersions key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"update_auto_check\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => {
                assert!(loaded.update_auto_check);
                assert_eq!(loaded.skipped_update_versions, vec!["1.0.0"]);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Phase 13 — `ai_features_enabled` defaults to true.
    #[test]
    fn ai_features_enabled_defaults_to_true() {
        let s = Settings::default();
        assert!(
            s.ai_features_enabled,
            "AI features ON by default per Phase 13 plan"
        );
    }

    /// Phase 13 — `ai_features_enabled` round-trips on the wire as
    /// camelCase `aiFeaturesEnabled`. Pin the wire shape so a future
    /// serde rename doesn't silently break the frontend store.
    #[tokio::test]
    async fn ai_features_enabled_round_trips_with_camel_case_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            ai_features_enabled: false,
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"aiFeaturesEnabled\""),
            "expected camelCase key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"ai_features_enabled\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert!(!loaded.ai_features_enabled),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Legacy `enhanced_trending_enabled` defaults to false. This is retained
    /// so old settings files do not accidentally enable unused network paths.
    #[test]
    fn enhanced_trending_defaults_to_false() {
        let s = Settings::default();
        assert!(
            !s.enhanced_trending_enabled,
            "enhanced trending must be OFF by default — endpoint is opt-in"
        );
    }

    /// v0.4.0 — older `settings.json` files written before the field
    /// existed must read cleanly with the field absent → false. Locks
    /// the forward-compat behaviour so a v0.3.x user upgrading to
    /// v0.4.0 gets the opt-in posture, not a silent enable.
    #[tokio::test]
    async fn missing_enhanced_trending_field_defaults_to_false() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Write a v0.3.x-shape settings.json with the new field absent.
        tokio::fs::write(
            &path,
            br#"{"paranoidMode": false, "catalogStaleBannerDays": 14}"#,
        )
        .await
        .unwrap();

        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => {
                assert!(
                    !s.enhanced_trending_enabled,
                    "missing field must default to false (opt-in posture)"
                );
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// v0.4.0 — `enhanced_trending_enabled` round-trips on the wire as
    /// camelCase `enhancedTrendingEnabled`. Pin the wire shape so a
    /// future serde rename doesn't silently break the frontend store.
    #[tokio::test]
    async fn enhanced_trending_round_trips_with_camel_case_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            enhanced_trending_enabled: true,
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"enhancedTrendingEnabled\""),
            "expected camelCase key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"enhanced_trending_enabled\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert!(loaded.enhanced_trending_enabled),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Legacy `vulnerability_scanning_enabled` defaults to false. Load-bearing:
    /// no scanner subprocess or enrichment traffic unless the user explicitly
    /// opts in to a future reworked feature.
    #[test]
    fn vulnerability_scanning_defaults_to_false() {
        let s = Settings::default();
        assert!(
            !s.vulnerability_scanning_enabled,
            "vulnerability scanning must be OFF by default — feature is opt-in"
        );
    }

    /// v0.5.0 — older `settings.json` files written before the field
    /// existed must read cleanly with the field absent → false. Locks the
    /// forward-compat behaviour so a v0.4.x user upgrading to v0.5.0 gets
    /// the opt-in posture, not a silent enable.
    #[tokio::test]
    async fn missing_vulnerability_scanning_field_defaults_to_false() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());
        // Write a v0.4.x-shape settings.json with the new field absent.
        tokio::fs::write(
            &path,
            br#"{"paranoidMode": false, "catalogStaleBannerDays": 14, "enhancedTrendingEnabled": true}"#,
        )
        .await
        .unwrap();

        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => {
                assert!(
                    !s.vulnerability_scanning_enabled,
                    "missing field must default to false (opt-in posture)"
                );
                // Sanity: the field present in the source file still loaded.
                assert!(s.enhanced_trending_enabled);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// v0.5.0 — `vulnerability_scanning_enabled` round-trips on the wire
    /// as camelCase `vulnerabilityScanningEnabled`. Pin the wire shape so
    /// a future serde rename doesn't silently break the frontend store.
    #[tokio::test]
    async fn vulnerability_scanning_round_trips_with_camel_case_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = Settings {
            vulnerability_scanning_enabled: true,
            ..Settings::default()
        };
        persist(tmp.path(), s.clone()).await.expect("persist");

        let raw = tokio::fs::read_to_string(settings_path(tmp.path()))
            .await
            .expect("read raw");
        assert!(
            raw.contains("\"vulnerabilityScanningEnabled\""),
            "expected camelCase key in raw JSON, got: {raw}"
        );
        assert!(
            !raw.contains("\"vulnerability_scanning_enabled\""),
            "must not emit snake_case key"
        );

        let reloaded = load_async(tmp.path()).await;
        match reloaded {
            SettingsLoadState::Loaded(loaded) => assert!(loaded.vulnerability_scanning_enabled),
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    /// Simulate a crash mid-write: write a `.tmp` file then truncate
    /// it. The final settings.json should remain whatever it was
    /// before (or absent), never the partial tmp contents.
    ///
    /// This exercises the atomic-write contract from `util::fs::atomic_write`:
    /// a crash before the `rename` step leaves the data file unchanged.
    #[tokio::test]
    async fn atomic_write_survives_simulated_crash() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());

        // 1. Establish a known-good initial state.
        let original = Settings::default();
        persist(tmp.path(), original.clone()).await.expect("seed");

        // 2. Simulate a crash mid-write by manually creating an
        // oversize / truncated .tmp sibling without renaming it. The
        // existence of `.tmp` must not pollute the final file.
        let mut tmp_name = path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp_sibling = std::path::PathBuf::from(tmp_name);
        tokio::fs::write(&tmp_sibling, b"\x00 partial garbage")
            .await
            .expect("write partial tmp");

        // 3. Read the final file — must still be the original payload.
        let state = load_at_startup(tmp.path());
        match state {
            SettingsLoadState::Loaded(s) => assert_eq!(s, original),
            other => panic!("expected Loaded with original, got {other:?}"),
        }
    }

    /// `settings_reset` overwrites a corrupt file with defaults.
    #[tokio::test]
    async fn reset_repairs_corrupt_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = settings_path(tmp.path());

        // Plant corrupt content.
        tokio::fs::write(&path, b"{ garbage").await.unwrap();
        let state_before = load_at_startup(tmp.path());
        assert!(matches!(state_before, SettingsLoadState::Corrupt { .. }));

        // Write defaults via persist (what settings_reset uses).
        let written = persist(tmp.path(), Settings::default())
            .await
            .expect("reset");
        assert_eq!(written, Settings::default());

        // Reload — must now be Loaded(defaults).
        let state_after = load_at_startup(tmp.path());
        match state_after {
            SettingsLoadState::Loaded(s) => assert_eq!(s, Settings::default()),
            other => panic!("expected Loaded after reset, got {other:?}"),
        }
    }

    /// effective_settings on FirstLaunch returns defaults (paranoid off).
    #[test]
    fn effective_settings_first_launch_returns_defaults() {
        let state = SettingsLoadState::FirstLaunch;
        let s = state
            .effective_settings()
            .expect("first launch yields defaults");
        assert_eq!(s, Settings::default());
        assert!(!s.paranoid_mode);
    }

    /// effective_settings on Corrupt returns None (fail closed signal).
    #[test]
    fn effective_settings_corrupt_returns_none() {
        let state = SettingsLoadState::Corrupt {
            message: "boom".into(),
        };
        assert!(state.effective_settings().is_none());
    }
}
