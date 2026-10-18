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
