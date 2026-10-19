use std::{fs, sync::Arc};

use csw_core::{
    native_code,
    platform::mock::MockPlatformProvider,
    profile::{ProfileManager, SharingConfig},
    switcher::ContextSwitcher,
};
use tempfile::TempDir;

const ACCOUNT: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
const ORG: &str = "33333333-3333-4333-8333-333333333333";
const SESSION: &str = "44444444-4444-4444-8444-444444444444";

struct Fixture {
    _tmp: TempDir,
    provider: Arc<MockPlatformProvider>,
    manager: Arc<ProfileManager>,
    original: std::path::PathBuf,
    other: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("desktop");
        let cli = tmp.path().join("cli");
        fs::create_dir_all(cli.join("projects")).unwrap();
        fs::write(cli.join("history.jsonl"), "history\n").unwrap();
        let provider = Arc::new(MockPlatformProvider::new(
            original.clone(),
            cli,
            tmp.path().join("switcher"),
        ));
        let manager = Arc::new(ProfileManager::new(provider.clone()).unwrap());
        let p = manager
            .create_profile("second", SharingConfig::share_workspace_preset(), None)
            .unwrap();
        let other = p.isolation.desktop_user_data_dir;
        for (root, account) in [(&original, ACCOUNT), (&other, OTHER)] {
            fs::create_dir_all(root.join("claude-code-sessions").join(account).join(ORG)).unwrap();
            fs::write(root.join("config.json"), format!("private-login-{account}")).unwrap();
            fs::write(root.join("Cookies"), format!("private-cookie-{account}")).unwrap();
        }
        Self {
            _tmp: tmp,
            provider,
            manager,
            original,
            other,
        }
    }
    fn bucket(&self, other: bool) -> std::path::PathBuf {
        (if other { &self.other } else { &self.original })
            .join("claude-code-sessions")
            .join(if other { OTHER } else { ACCOUNT })
            .join(ORG)
    }
    fn enable(&self) {
        native_code::configure(
            self.manager.as_ref(),
            self.provider.as_ref(),
            "default",
            &["second".into()],
        )
        .unwrap();
    }
}

#[test]
fn default_code_preset_works_without_cli_signin_or_copying_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let cli = tmp.path().join("cli");
    let provider = Arc::new(MockPlatformProvider::new(
        tmp.path().join("desktop"),
        cli.clone(),
        tmp.path().join("switcher"),
    ));
    let manager = ProfileManager::new(provider).unwrap();
    let p = manager
        .create_profile("fresh", SharingConfig::share_code_preset(), None)
        .unwrap();
    assert!(cli.join("projects").is_dir());
    assert!(cli.join("history.jsonl").is_file());
    assert_eq!(
        p.isolation
            .cli_config_dir
            .join("projects")
            .canonicalize()
            .unwrap(),
        cli.join("projects").canonicalize().unwrap()
    );
    assert_eq!(
        p.isolation
            .cli_config_dir
            .join("history.jsonl")
            .canonicalize()
            .unwrap(),
        cli.join("history.jsonl").canonicalize().unwrap()
    );
    for file in [".credentials.json", "settings.json", "CLAUDE.md"] {
        assert!(!cli.join(file).exists());
        assert!(!p.isolation.cli_config_dir.join(file).exists());
    }
    assert!(!p.sharing.is_fully_isolated());
}

#[test]
fn gui_enrollment_seeds_first_member_after_confirmation_and_preserves_logins() {
    let f = Fixture::new();
    fs::write(
        f.bucket(false).join("original.json"),
        r#"{"original":true}"#,
    )
    .unwrap();
    fs::write(f.bucket(true).join("prior.json"), r#"{"prior":true}"#).unwrap();
    assert!(
        native_code::add_member(f.manager.as_ref(), f.provider.as_ref(), "second", false).is_err()
    );
    assert!(!f.provider.app_data.join("native-code.toml").exists());
    let backup =
        native_code::add_member(f.manager.as_ref(), f.provider.as_ref(), "second", true).unwrap();
    assert!(backup.join("1/bucket/prior.json").is_file());
    assert!(f.bucket(true).join("original.json").is_file());
    assert_eq!(
        fs::read_to_string(f.other.join("config.json")).unwrap(),
        format!("private-login-{OTHER}")
    );
    let st =
        native_code::sharing_status(f.manager.as_ref(), f.provider.as_ref(), "second").unwrap();
    assert!(st.enabled);
    assert_eq!(st.participants, 2);
    assert!(
        native_code::add_member(f.manager.as_ref(), f.provider.as_ref(), "second", true).is_err()
    );
}

fn third_environment(f: &Fixture) -> std::path::PathBuf {
    let p = f
        .manager
        .create_profile("third", SharingConfig::share_code_preset(), None)
        .unwrap();
    let bucket = p
        .isolation
        .desktop_user_data_dir
        .join("claude-code-sessions")
        .join(SESSION)
        .join(ORG);
    fs::create_dir_all(&bucket).unwrap();
    bucket
}

#[test]
fn adding_member_keeps_existing_members_and_their_latest_changes() {
    let f = Fixture::new();
    f.enable();
    let third = third_environment(&f);
    fs::write(f.bucket(true).join("latest.json"), r#"{"latest":true}"#).unwrap();
    fs::write(third.join("old.json"), r#"{"old":true}"#).unwrap();
    native_code::add_member(f.manager.as_ref(), f.provider.as_ref(), "third", true).unwrap();
    for bucket in [f.bucket(false), f.bucket(true), third] {
        assert!(bucket.join("latest.json").is_file());
        assert!(!bucket.join("old.json").exists());
    }
    let st =
        native_code::sharing_status(f.manager.as_ref(), f.provider.as_ref(), "second").unwrap();
    assert!(st.enabled);
    assert_eq!(st.participants, 3);
}

#[test]
fn enrollment_refuses_conflicting_existing_changes_without_seeding_new_member() {
    let f = Fixture::new();
    f.enable();
    let third = third_environment(&f);
    fs::write(f.bucket(false).join("one.json"), "{}").unwrap();
    fs::write(f.bucket(true).join("two.json"), "{}").unwrap();
    fs::write(third.join("keep.json"), "{}").unwrap();
    let config = fs::read(f.provider.app_data.join("native-code.toml")).unwrap();
    assert!(
        native_code::add_member(f.manager.as_ref(), f.provider.as_ref(), "third", true).is_err()
    );
    assert_eq!(
        fs::read(f.provider.app_data.join("native-code.toml")).unwrap(),
        config
    );
    assert!(third.join("keep.json").is_file());
    assert!(!third.join("one.json").exists());
}

#[test]
fn enrollment_never_runs_with_desktop_or_input_state_unknown() {
    let f = Fixture::new();
    f.provider.set_desktop_state(true, vec![]);
    assert!(
        native_code::add_member(f.manager.as_ref(), f.provider.as_ref(), "second", true).is_err()
    );
    f.provider
        .set_desktop_state(false, vec![(123, "Claude --user-data-dir=/another".into())]);
    assert!(
        native_code::add_member(f.manager.as_ref(), f.provider.as_ref(), "second", true).is_err()
    );
    assert!(!f.provider.app_data.join("native-code.toml").exists());
}

#[test]
fn complete_native_records_and_worktree_state_follow_switches_without_copying_logins() {
    let f = Fixture::new();
    let name = format!("local_{SESSION}.json");
    let body =
        r#"{"title":"Original","arbitraryFutureField":{"permission":"keep me"},"isArchived":true}"#;
    fs::write(f.bucket(false).join(&name), body).unwrap();
    fs::write(f.original.join("git-worktrees.json"), r#"{"schemaVersion":2,"worktrees":{"branch":{"leasedBy":"session","path":"/repo"}},"untrackedDirGc":{"roots":{}}}"#).unwrap();
    fs::write(
        f.original.join("ccd-session-presets.json"),
        format!(r#"{{"byAccount":{{"{ACCOUNT}":{{"presets":[{{"name":"original"}}]}}}}}}"#),
    )
    .unwrap();
    fs::write(f.bucket(true).join("old.json"), r#"{"old":true}"#).unwrap();
    let auth: Vec<_> = [&f.original, &f.other]
        .into_iter()
        .map(|p| {
            (
                fs::read(p.join("config.json")).unwrap(),
                fs::read(p.join("Cookies")).unwrap(),
            )
        })
        .collect();
    f.enable();
    assert_eq!(
        fs::read_to_string(f.bucket(true).join(&name)).unwrap(),
        body
    );
    assert!(!f.bucket(true).join("old.json").exists());
    assert_eq!(
        fs::read(f.original.join("git-worktrees.json")).unwrap(),
        fs::read(f.other.join("git-worktrees.json")).unwrap()
    );
    let presets: serde_json::Value =
        serde_json::from_slice(&fs::read(f.other.join("ccd-session-presets.json")).unwrap())
            .unwrap();
    assert_eq!(
        presets["byAccount"][OTHER]["presets"][0]["name"],
        "original"
    );
    let switcher = ContextSwitcher::new(f.provider.clone(), f.manager.clone());
    switcher.switch_to("second").unwrap();
    fs::remove_file(f.bucket(true).join(&name)).unwrap();
    fs::write(
        f.bucket(true).join(format!("deleted_{SESSION}")),
        "1800000000000",
    )
    .unwrap();
    fs::write(
        f.other.join("git-worktrees.json"),
        "{\"schemaVersion\":2,\"fullUpdatedStore\":true}",
    )
    .unwrap();
    switcher.switch_to("default").unwrap();
    assert!(!f.bucket(false).join(name).exists());
    assert!(f.bucket(false).join(format!("deleted_{SESSION}")).exists());
    assert_eq!(
        fs::read(f.original.join("git-worktrees.json")).unwrap(),
        fs::read(f.other.join("git-worktrees.json")).unwrap()
    );
    for (i, p) in [&f.original, &f.other].into_iter().enumerate() {
        assert_eq!(fs::read(p.join("config.json")).unwrap(), auth[i].0);
        assert_eq!(fs::read(p.join("Cookies")).unwrap(), auth[i].1);
    }
}

#[test]
fn ambiguous_account_buckets_are_rejected_before_overwriting_anything() {
    let f = Fixture::new();
    fs::create_dir_all(f.other.join("claude-code-sessions").join(ACCOUNT).join(ORG)).unwrap();
    fs::write(f.bucket(true).join("sentinel.json"), "preserve").unwrap();
    assert!(
        native_code::configure(
            f.manager.as_ref(),
            f.provider.as_ref(),
            "default",
            &["second".into()]
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(f.bucket(true).join("sentinel.json")).unwrap(),
        "preserve"
    );
}

#[test]
fn independent_changes_are_preserved_and_require_an_explicit_source_choice() {
    let f = Fixture::new();
    f.enable();
    fs::write(f.bucket(false).join("one.json"), "{\"first\":true}").unwrap();
    fs::write(f.bucket(true).join("two.json"), "{\"second\":true}").unwrap();
    let switcher = ContextSwitcher::new(f.provider.clone(), f.manager.clone());
    assert!(switcher.switch_to("second").is_err());
    assert_eq!(f.manager.active_profile_name(), "default");
    assert!(f.bucket(false).join("one.json").exists());
    assert!(f.bucket(true).join("two.json").exists());
    assert!(!f.bucket(false).join("two.json").exists());
}

#[test]
fn a_manually_opened_original_is_detected_without_relying_on_the_active_marker() {
    let f = Fixture::new();
    f.enable();
    let switcher = ContextSwitcher::new(f.provider.clone(), f.manager.clone());
    switcher.switch_to("second").unwrap();
    fs::write(f.bucket(false).join("new.json"), "{\"fromDock\":true}").unwrap();
    switcher.switch_to("default").unwrap();
    assert_eq!(
        fs::read(f.bucket(false).join("new.json")).unwrap(),
        fs::read(f.bucket(true).join("new.json")).unwrap()
    );
}

#[test]
fn running_desktop_blocks_seeding_before_any_code_data_is_replaced() {
    let f = Fixture::new();
    let running = MockPlatformProvider::new(
        f.original.clone(),
        f.provider.cli_default.clone(),
        f.provider.app_data.clone(),
    )
    .with_desktop_running(true);
    fs::write(f.bucket(true).join("keep.json"), "{\"keep\":true}").unwrap();
    assert!(
        native_code::configure(f.manager.as_ref(), &running, "default", &["second".into()])
            .is_err()
    );
    assert!(f.bucket(true).join("keep.json").exists());
    assert!(!f.provider.app_data.join("native-code.toml").exists());
}

#[cfg(unix)]
#[test]
fn symlinks_and_hardlinks_in_a_store_never_copy_an_external_file() {
    let f = Fixture::new();
    let secret = f._tmp.path().join("outside-secret");
    fs::write(&secret, r#"{"secret":"must remain outside"}"#).unwrap();
    let planted = f.bucket(false).join("planted.json");
    std::os::unix::fs::symlink(&secret, &planted).unwrap();
    assert!(
        native_code::configure(
            f.manager.as_ref(),
            f.provider.as_ref(),
            "default",
            &["second".into()]
        )
        .is_err()
    );
    fs::remove_file(&planted).unwrap();
    fs::hard_link(&secret, &planted).unwrap();
    assert!(
        native_code::configure(
            f.manager.as_ref(),
            f.provider.as_ref(),
            "default",
            &["second".into()]
        )
        .is_err()
    );
    assert!(!f.bucket(true).join("planted.json").exists());
}

#[test]
fn removing_account_presets_propagates_back_to_the_original() {
    let f = Fixture::new();
    fs::write(
        f.original.join("ccd-session-presets.json"),
        format!(r#"{{"byAccount":{{"{ACCOUNT}":{{"name":"old"}}}}}}"#),
    )
    .unwrap();
    f.enable();
    let path = f.other.join("ccd-session-presets.json");
    let mut presets: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    presets["byAccount"].as_object_mut().unwrap().remove(OTHER);
    fs::write(path, serde_json::to_vec(&presets).unwrap()).unwrap();
    let switcher = ContextSwitcher::new(f.provider.clone(), f.manager.clone());
    switcher.switch_to("default").unwrap();
    let presets: serde_json::Value =
        serde_json::from_slice(&fs::read(f.original.join("ccd-session-presets.json")).unwrap())
            .unwrap();
    assert!(presets["byAccount"].get(ACCOUNT).is_none());
}

#[test]
fn deleting_a_participant_requires_disabling_sharing_and_keeps_data() {
    let f = Fixture::new();
    f.enable();
    assert!(f.manager.delete_profile("second").is_err());
    assert!(f.manager.purge_profile("second").is_err());
    assert!(f.other.exists());
    native_code::disable(f.manager.as_ref(), f.provider.as_ref()).unwrap();
    assert!(f.other.exists());
    f.manager.delete_profile("second").unwrap();
    ContextSwitcher::new(f.provider.clone(), f.manager.clone())
        .switch_to("default")
        .unwrap();
}

// Reconstruct a durable transaction exactly as a process killed between writes
// leaves it. These fixtures never open real Claude data or authentication files.
fn pending_journal(f: &Fixture, backup: &std::path::Path, prior: Option<&str>, planned: &str) {
    let group: toml::Value = toml::from_str(planned).unwrap();
    let journal = serde_json::json!({
        "version": 1, "tag": backup.file_name().unwrap().to_str().unwrap(), "authority":"default",
        "group": group, "prior_config": prior, "planned_config": planned,
        "places": [["default", f.original, f.bucket(false)], ["second", f.other, f.bucket(true)]]
    });
    fs::write(
        f.provider.app_data.join("native-code-journal.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
}

#[test]
fn interrupted_initial_seeding_recovers_missing_bucket_and_partial_global_files() {
    for missing_bucket in [false, true] {
        let f = Fixture::new();
        fs::write(f.bucket(false).join("seed.json"), r#"{"source":true}"#).unwrap();
        fs::write(f.bucket(true).join("original.json"), r#"{"restore":true}"#).unwrap();
        fs::write(f.other.join("git-worktrees.json"), r#"{"original":true}"#).unwrap();
        let backup = native_code::configure(
            f.manager.as_ref(),
            f.provider.as_ref(),
            "default",
            &["second".into()],
        )
        .unwrap();
        let config = f.provider.app_data.join("native-code.toml");
        let planned = fs::read_to_string(&config).unwrap();
        fs::remove_file(&config).unwrap();
        if missing_bucket {
            fs::rename(
                f.bucket(true),
                f.bucket(true).with_file_name(format!(
                    ".csw-previous-{}",
                    backup.file_name().unwrap().to_str().unwrap()
                )),
            )
            .unwrap();
        } else {
            fs::write(f.other.join("git-worktrees.json"), r#"{"original":true}"#).unwrap();
        }
        pending_journal(&f, &backup, None, &planned);
        assert!(native_code::is_shared(f.provider.as_ref(), "second").is_err());
        ContextSwitcher::new(f.provider.clone(), f.manager.clone())
            .switch_to("default")
            .unwrap();
        assert!(!f.bucket(true).join("original.json").exists());
        assert!(f.bucket(true).join("seed.json").exists());
        assert!(!f.other.join("git-worktrees.json").exists());
        assert!(config.exists());
        assert_eq!(
            fs::read_to_string(backup.join("1/bucket/original.json")).unwrap(),
            r#"{"restore":true}"#
        );
        assert!(
            !f.provider
                .app_data
                .join("native-code-journal.json")
                .exists()
        );
    }
}

#[test]
fn committed_journal_keeps_new_stores_and_only_cleans_transaction_debris() {
    let f = Fixture::new();
    fs::write(f.bucket(false).join("seed.json"), r#"{"seed":true}"#).unwrap();
    let backup = native_code::configure(
        f.manager.as_ref(),
        f.provider.as_ref(),
        "default",
        &["second".into()],
    )
    .unwrap();
    let planned = fs::read_to_string(f.provider.app_data.join("native-code.toml")).unwrap();
    pending_journal(&f, &backup, None, &planned);
    fs::write(
        f.bucket(true).join("after-commit.json"),
        r#"{"later":true}"#,
    )
    .unwrap();
    let debris = f.bucket(true).with_file_name(format!(
        ".csw-stage-{}",
        backup.file_name().unwrap().to_str().unwrap()
    ));
    fs::create_dir(&debris).unwrap();
    ContextSwitcher::new(f.provider.clone(), f.manager.clone())
        .switch_to("second")
        .unwrap();
    assert!(f.bucket(true).join("seed.json").exists());
    assert!(f.bucket(false).join("after-commit.json").exists());
    assert!(!debris.exists());
    assert!(
        !f.provider
            .app_data
            .join("native-code-journal.json")
            .exists()
    );
}

#[test]
fn interrupted_update_completes_the_planned_source_and_preserves_the_exact_prior_backup() {
    let f = Fixture::new();
    f.enable();
    let config = f.provider.app_data.join("native-code.toml");
    let prior = format!(
        "# preserve user comments\n{}",
        fs::read_to_string(&config).unwrap()
    );
    fs::write(&config, &prior).unwrap();
    fs::write(f.bucket(false).join("new.json"), r#"{"new":true}"#).unwrap();
    // A completed update supplies durable originals and planned fingerprints.
    let backup = native_code::configure(
        f.manager.as_ref(),
        f.provider.as_ref(),
        "default",
        &["second".into()],
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(backup.join("native-code.toml")).unwrap(),
        prior
    );
    let planned = fs::read_to_string(&config).unwrap();
    fs::write(&config, &prior).unwrap();
    pending_journal(&f, &backup, Some(&prior), &planned);
    // Disable first completes the interrupted update, then removes only config.
    native_code::disable(f.manager.as_ref(), f.provider.as_ref()).unwrap();
    assert!(f.bucket(false).join("new.json").exists());
    assert!(f.bucket(true).join("new.json").exists());
    assert!(!config.exists());
}

#[test]
fn interrupted_reseed_keeps_the_explicit_source_even_when_it_did_not_change() {
    let f = Fixture::new();
    fs::write(f.bucket(false).join("seed.json"), r#"{"seed":true}"#).unwrap();
    f.enable();
    let config = f.provider.app_data.join("native-code.toml");
    let prior = fs::read_to_string(&config).unwrap();
    fs::write(f.bucket(true).join("discard.json"), r#"{"discard":true}"#).unwrap();
    let backup = native_code::configure(
        f.manager.as_ref(),
        f.provider.as_ref(),
        "default",
        &["second".into()],
    )
    .unwrap();
    let planned = fs::read_to_string(&config).unwrap();
    assert_ne!(prior, planned);
    // Recreate termination before the destination was replaced.
    fs::write(&config, &prior).unwrap();
    fs::write(f.bucket(true).join("discard.json"), r#"{"discard":true}"#).unwrap();
    pending_journal(&f, &backup, Some(&prior), &planned);
    ContextSwitcher::new(f.provider.clone(), f.manager.clone())
        .switch_to("second")
        .unwrap();
    assert!(!f.bucket(false).join("discard.json").exists());
    assert!(!f.bucket(true).join("discard.json").exists());
    assert!(backup.join("1/bucket/discard.json").exists());
}

#[test]
fn launch_reservation_serializes_reseed_and_survives_an_interrupted_launcher() {
    let f = Fixture::new();
    f.enable();
    let guard = native_code::launch_guard(
        f.manager.as_ref(),
        f.provider.as_ref(),
        &f.manager.get_profile("default").unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(
        native_code::configure(
            f.manager.as_ref(),
            f.provider.as_ref(),
            "default",
            &["second".into()]
        )
        .is_err()
    );
    drop(guard); // Simulate a launcher exiting before observing Claude startup.
    assert!(
        native_code::configure(
            f.manager.as_ref(),
            f.provider.as_ref(),
            "default",
            &["second".into()]
        )
        .is_err()
    );
    native_code::disable(f.manager.as_ref(), f.provider.as_ref()).unwrap();
    f.enable();
    let guard = native_code::launch_guard(
        f.manager.as_ref(),
        f.provider.as_ref(),
        &f.manager.get_profile("default").unwrap(),
    )
    .unwrap()
    .unwrap();
    let running = MockPlatformProvider::new(
        f.original.clone(),
        f.provider.cli_default.clone(),
        f.provider.app_data.clone(),
    )
    .with_desktop_running(true);
    running.set_desktop_state(true, vec![(123, "Claude".into())]);
    native_code::finish_launch(f.manager.as_ref(), &running, &guard).unwrap();
    drop(guard);
    assert!(
        !f.provider
            .app_data
            .join("native-code-launch-pending")
            .exists()
    );
    assert!(
        native_code::launch_guard(
            f.manager.as_ref(),
            &running,
            &f.manager.get_profile("second").unwrap()
        )
        .is_err()
    );
}

#[test]
fn housekeeping_on_a_stale_member_does_not_block_latest_content_or_revive_deleted_threads() {
    let f = Fixture::new();
    let session = format!("local_{SESSION}.json");
    fs::write(
        f.bucket(false).join(&session),
        r#"{"title":"old","lastFocusedAt":10,"future":{"keep":true}}"#,
    )
    .unwrap();
    fs::write(f.bucket(false).join("removed.json"), "{}").unwrap();
    fs::write(f.original.join("git-worktrees.json"), r#"{"worktrees":{"repo":{"branch":"old"}},"untrackedDirGc":{"cwds":{"/synthetic/repo":10}}}"#).unwrap();
    f.enable();
    // Legacy configuration has no confirmed launch provenance yet. The old
    // implementation mistakes these timestamp writes for competing content.
    fs::write(
        f.bucket(false).join(&session),
        r#"{"title":"old","lastFocusedAt":900,"future":{"keep":true}}"#,
    )
    .unwrap();
    fs::write(f.original.join("git-worktrees.json"), r#"{"worktrees":{"repo":{"branch":"old"}},"untrackedDirGc":{"cwds":{"/synthetic/repo":900}}}"#).unwrap();
    fs::write(
        f.bucket(true).join(&session),
        r#"{"title":"latest","lastFocusedAt":20,"future":{"keep":true}}"#,
    )
    .unwrap();
    fs::remove_file(f.bucket(true).join("removed.json")).unwrap();
    fs::write(f.bucket(true).join("deleted_synthetic"), "1234").unwrap();
    fs::write(f.other.join("git-worktrees.json"), r#"{"worktrees":{"repo":{"branch":"latest"}},"untrackedDirGc":{"cwds":{"/synthetic/repo":20}}}"#).unwrap();
    ContextSwitcher::new(f.provider.clone(), f.manager.clone())
        .switch_to("default")
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(f.bucket(false).join(&session)).unwrap()).unwrap();
    assert_eq!(v["title"], "latest");
    assert_eq!(v["lastFocusedAt"], 900);
    assert_eq!(v["future"]["keep"], true);
    assert!(!f.bucket(false).join("removed.json").exists());
    assert!(f.bucket(false).join("deleted_synthetic").exists());
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(f.original.join("git-worktrees.json")).unwrap()).unwrap();
    assert_eq!(v["worktrees"]["repo"]["branch"], "latest");
    assert_eq!(v["untrackedDirGc"]["cwds"]["/synthetic/repo"], 900);
}

fn confirm_open(f: &Fixture, name: &str) {
    let profile = f.manager.get_profile(name).unwrap();
    let guard = native_code::launch_guard(f.manager.as_ref(), f.provider.as_ref(), &profile)
        .unwrap()
        .unwrap();
    f.provider.set_desktop_state(
        true,
        vec![(
            123,
            format!(
                "Claude --user-data-dir={}",
                profile.isolation.desktop_user_data_dir.display()
            ),
        )],
    );
    native_code::finish_launch(f.manager.as_ref(), f.provider.as_ref(), &guard).unwrap();
    drop(guard);
    f.provider.set_desktop_state(false, vec![]);
}

fn code_group(f: &Fixture) -> toml::Value {
    toml::from_str(&fs::read_to_string(f.provider.app_data.join("native-code.toml")).unwrap())
        .unwrap()
}

#[test]
fn confirmed_launch_beats_active_marker_focus_and_known_stale_destination() {
    let f = Fixture::new();
    let name = format!("local_{SESSION}.json");
    fs::write(
        f.bucket(false).join(&name),
        r#"{"title":"v1","lastFocusedAt":1}"#,
    )
    .unwrap();
    f.enable();
    let stale = fs::read(f.bucket(false).join(&name)).unwrap();
    confirm_open(&f, "second");
    fs::write(
        f.bucket(true).join(&name),
        r#"{"title":"v2","lastFocusedAt":2}"#,
    )
    .unwrap();
    ContextSwitcher::new(f.provider.clone(), f.manager.clone())
        .switch_to("default")
        .unwrap();
    // A switch without an observed launch must leave authority on second.
    assert_eq!(
        code_group(&f)["last_launch"]["name"].as_str(),
        Some("second")
    );
    fs::write(f.bucket(false).join(&name), stale).unwrap();
    fs::write(
        f.bucket(true).join(&name),
        r#"{"title":"v3","lastFocusedAt":3}"#,
    )
    .unwrap();
    let manager = Arc::new(ProfileManager::new(f.provider.clone()).unwrap());
    ContextSwitcher::new(f.provider.clone(), manager)
        .switch_to("default")
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(f.bucket(false).join(&name)).unwrap()).unwrap();
    assert_eq!(v["title"], "v3");
    confirm_open(&f, "default");
    fs::remove_file(f.bucket(false).join(&name)).unwrap();
    fs::write(f.bucket(false).join("deleted_thread"), "20").unwrap();
    ContextSwitcher::new(f.provider.clone(), f.manager.clone())
        .switch_to("second")
        .unwrap();
    assert!(!f.bucket(true).join(name).exists());
    assert!(f.bucket(true).join("deleted_thread").exists());
}

#[test]
fn unknown_external_writer_never_overwrites_confirmed_authority() {
    let f = Fixture::new();
    f.enable();
    confirm_open(&f, "second");
    fs::write(f.bucket(false).join("external.json"), r#"{"unknown":true}"#).unwrap();
    let config = fs::read(f.provider.app_data.join("native-code.toml")).unwrap();
    let error = ContextSwitcher::new(f.provider.clone(), f.manager.clone())
        .switch_to("default")
        .unwrap_err();
    assert!(matches!(
        error,
        csw_core::error::CswError::LocalCodeConflict(_)
    ));
    assert_eq!(
        csw_core::switcher::pending::launch_error_code(&error),
        "history_conflict"
    );
    assert_eq!(
        config,
        fs::read(f.provider.app_data.join("native-code.toml")).unwrap()
    );
    assert!(!f.bucket(true).join("external.json").exists());
}

#[test]
fn wrong_process_and_interrupted_launch_do_not_advance_authority() {
    let f = Fixture::new();
    f.enable();
    confirm_open(&f, "default");
    let profile = f.manager.get_profile("second").unwrap();
    let guard = native_code::launch_guard(f.manager.as_ref(), f.provider.as_ref(), &profile)
        .unwrap()
        .unwrap();
    f.provider
        .set_desktop_state(true, vec![(1, "Claude".into())]);
    assert!(native_code::finish_launch(f.manager.as_ref(), f.provider.as_ref(), &guard).is_err());
    assert_eq!(
        code_group(&f)["last_launch"]["name"].as_str(),
        Some("default")
    );
    drop(guard);
    f.provider.set_desktop_state(false, vec![]);
    assert!(native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "second").is_err());
    assert!(
        f.provider
            .app_data
            .join("native-code-launch-pending")
            .exists()
    );
}

#[test]
fn completed_launch_commit_can_clean_its_exact_leftover_reservation() {
    let f = Fixture::new();
    f.enable();
    let profile = f.manager.get_profile("second").unwrap();
    let guard = native_code::launch_guard(f.manager.as_ref(), f.provider.as_ref(), &profile)
        .unwrap()
        .unwrap();
    let pending = f.provider.app_data.join("native-code-launch-pending");
    let bytes = fs::read(&pending).unwrap();
    f.provider.set_desktop_state(
        true,
        vec![(5, format!("Claude --user-data-dir={}", f.other.display()))],
    );
    native_code::finish_launch(f.manager.as_ref(), f.provider.as_ref(), &guard).unwrap();
    drop(guard);
    f.provider.set_desktop_state(false, vec![]);
    fs::write(&pending, bytes).unwrap();
    native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "default").unwrap();
    assert!(!pending.exists());
    assert_eq!(
        code_group(&f)["last_launch"]["name"].as_str(),
        Some("second")
    );
}

#[test]
fn legacy_baseline_is_verified_and_housekeeping_does_not_create_a_conflict() {
    let f = Fixture::new();
    let name = format!("local_{SESSION}.json");
    fs::write(
        f.bucket(false).join(&name),
        r#"{"title":"base","lastFocusedAt":1}"#,
    )
    .unwrap();
    f.enable();
    let mut legacy = code_group(&f);
    legacy["version"] = 1.into();
    let m = legacy.as_table_mut().unwrap();
    for k in [
        "content_fingerprints",
        "known_content",
        "identities",
        "last_launch",
    ] {
        m.remove(k);
    }
    fs::write(
        f.provider.app_data.join("native-code.toml"),
        toml::to_string(&legacy).unwrap(),
    )
    .unwrap();
    fs::write(
        f.bucket(false).join(&name),
        r#"{"title":"base","lastFocusedAt":1000}"#,
    )
    .unwrap();
    fs::write(
        f.bucket(true).join(&name),
        r#"{"title":"new","lastFocusedAt":2}"#,
    )
    .unwrap();
    native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "default").unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(f.bucket(false).join(&name)).unwrap()).unwrap();
    assert_eq!(value["title"], "new");
    assert_eq!(value["lastFocusedAt"], 1000);
    assert_eq!(code_group(&f)["version"].as_integer(), Some(2));
}

#[test]
fn recovery_preserves_unknown_post_crash_content_instead_of_replaying_over_it() {
    let f = Fixture::new();
    fs::write(f.bucket(false).join("seed.json"), "{}").unwrap();
    let backup = native_code::configure(
        f.manager.as_ref(),
        f.provider.as_ref(),
        "default",
        &["second".into()],
    )
    .unwrap();
    let config = f.provider.app_data.join("native-code.toml");
    let planned = fs::read_to_string(&config).unwrap();
    fs::remove_file(&config).unwrap();
    fs::write(f.bucket(true).join("external.json"), "{}").unwrap();
    pending_journal(&f, &backup, None, &planned);
    let error =
        native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "default").unwrap_err();
    assert!(matches!(
        error,
        csw_core::error::CswError::LocalCodeRecoveryConflict(_)
    ));
    assert!(f.bucket(true).join("external.json").exists());
    assert!(
        f.provider
            .app_data
            .join("native-code-journal.json")
            .exists()
    );
    assert!(!config.exists());
}

#[test]
fn nested_focus_and_unknown_gc_fields_remain_substantive_content() {
    for file in ["session.json", "git-worktrees.json"] {
        let f = Fixture::new();
        fs::write(
            f.bucket(false).join("session.json"),
            r#"{"nested":{"lastFocusedAt":1}}"#,
        )
        .unwrap();
        fs::write(
            f.original.join("git-worktrees.json"),
            r#"{"untrackedDirGc":{"roots":{"keep":1},"cwds":{}}}"#,
        )
        .unwrap();
        f.enable();
        confirm_open(&f, "second");
        if file == "session.json" {
            fs::write(
                f.bucket(false).join(file),
                r#"{"nested":{"lastFocusedAt":2}}"#,
            )
            .unwrap();
        } else {
            fs::write(
                f.original.join(file),
                r#"{"untrackedDirGc":{"roots":{"keep":2},"cwds":{}}}"#,
            )
            .unwrap();
        }
        assert!(
            native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "default").is_err()
        );
    }
}

#[test]
fn identity_change_is_refused_before_any_destination_is_replaced() {
    let f = Fixture::new();
    f.enable();
    confirm_open(&f, "second");
    let account = f.bucket(true).parent().unwrap().to_path_buf();
    fs::rename(&account, account.with_file_name(SESSION)).unwrap();
    let config = fs::read(f.provider.app_data.join("native-code.toml")).unwrap();
    assert!(
        native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "default").is_err()
    );
    assert_eq!(
        config,
        fs::read(f.provider.app_data.join("native-code.toml")).unwrap()
    );
    assert!(f.bucket(false).exists());
}

#[test]
fn isolated_launch_is_serialized_without_taking_shared_history_authority() {
    let f = Fixture::new();
    f.enable();
    confirm_open(&f, "second");
    f.manager
        .create_profile("isolated", SharingConfig::default(), None)
        .unwrap();
    confirm_open(&f, "isolated");
    assert_eq!(
        code_group(&f)["last_launch"]["name"].as_str(),
        Some("second")
    );
}

#[cfg(unix)]
#[test]
fn untrusted_state_files_and_overdeep_stores_are_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.enable();
    let config = f.provider.app_data.join("native-code.toml");
    let external = f._tmp.path().join("external");
    fs::rename(&config, &external).unwrap();
    symlink(&external, &config).unwrap();
    assert!(native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "second").is_err());
    fs::remove_file(&config).unwrap();
    fs::hard_link(&external, &config).unwrap();
    assert!(native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "second").is_err());
    fs::remove_file(&config).unwrap();
    fs::rename(&external, &config).unwrap();
    let deep = f
        .bucket(true)
        .join(std::iter::repeat_n("dir", 66).collect::<std::path::PathBuf>());
    fs::create_dir_all(deep).unwrap();
    assert!(native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "second").is_err());
}

#[test]
fn a_real_previous_version_journal_recovers_without_new_schema_fields() {
    let f = Fixture::new();
    fs::write(f.bucket(false).join("seed.json"), "{}").unwrap();
    let backup = native_code::configure(
        f.manager.as_ref(),
        f.provider.as_ref(),
        "default",
        &["second".into()],
    )
    .unwrap();
    let group = code_group(&f);
    // Literal old Group shape, rather than the new serializer's default fields.
    let planned = format!(
        "version = 1\ngeneration = \"{}\"\nsource = \"default\"\nmembers = [\"second\"]\n\n[fingerprints]\ndefault = \"{}\"\nsecond = \"{}\"\n",
        group["generation"].as_str().unwrap(),
        group["fingerprints"]["default"].as_str().unwrap(),
        group["fingerprints"]["second"].as_str().unwrap()
    );
    fs::remove_file(f.provider.app_data.join("native-code.toml")).unwrap();
    pending_journal(&f, &backup, None, &planned);
    native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "second").unwrap();
    assert!(f.bucket(true).join("seed.json").exists());
    assert!(
        !f.provider
            .app_data
            .join("native-code-journal.json")
            .exists()
    );
    assert_eq!(code_group(&f)["version"].as_integer(), Some(2));
}

#[test]
fn revisited_previous_snapshot_survives_known_history_rollover() {
    let f = Fixture::new();
    f.enable();
    confirm_open(&f, "second");
    let mut group = code_group(&f);
    let previous = group["content_fingerprints"]["default"]
        .as_str()
        .unwrap()
        .to_string();
    let mut known = vec![toml::Value::String(previous.clone())];
    known.extend((0..15).map(|i| toml::Value::String(format!("{i:064x}"))));
    group["known_content"]["default"] = toml::Value::Array(known);
    fs::write(
        f.provider.app_data.join("native-code.toml"),
        toml::to_string(&group).unwrap(),
    )
    .unwrap();
    fs::write(f.bucket(true).join("latest.json"), "{}").unwrap();
    native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "default").unwrap();
    let group = code_group(&f);
    let known = group["known_content"]["default"].as_array().unwrap();
    assert_eq!(known.len(), 16);
    assert!(known.iter().any(|k| k.as_str() == Some(&previous)));
    // A destination reverting to the just-previous empty store is recognized.
    fs::remove_file(f.bucket(false).join("latest.json")).unwrap();
    native_code::before_switch(f.manager.as_ref(), f.provider.as_ref(), "default").unwrap();
    assert!(f.bucket(false).join("latest.json").exists());
}
