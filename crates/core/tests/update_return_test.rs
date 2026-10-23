use csw_core::update_return::UpdateReturnWatch;
use std::time::{Duration, Instant};

#[test]
fn installed_update_reopens_the_observed_environment_once() {
    let mut watch = UpdateReturnWatch::default();
    let now = Instant::now();
    assert!(
        watch
            .observe(
                now,
                &[(10, "Claude --user-data-dir=/named".into())],
                Some(("Chris", "identity")),
                "old"
            )
            .is_none()
    );
    assert!(
        watch
            .observe(now + Duration::from_secs(3), &[], None, "new")
            .is_none()
    );
    let plan = watch
        .observe(
            now + Duration::from_secs(6),
            &[(20, "Claude".into())],
            None,
            "new",
        )
        .unwrap();
    assert_eq!(plan.target, "Chris");
    assert_eq!(plan.identity, "identity");
    assert_eq!(plan.pid, 20);
    assert!(
        watch
            .observe(
                now + Duration::from_secs(9),
                &[(20, "Claude".into())],
                None,
                "new"
            )
            .is_none()
    );
}

#[test]
fn normal_launches_ambiguous_snapshots_and_expired_transitions_are_untouched() {
    let now = Instant::now();
    for condition in [
        "unchanged_app",
        "same_pid",
        "multiple",
        "unknown",
        "expired",
        "restart",
        "inspection_gap",
    ] {
        let mut watch = UpdateReturnWatch::default();
        if condition != "restart" {
            watch.observe(
                now,
                &[(10, "Claude --user-data-dir=/named".into())],
                Some(("Chris", "identity")),
                "old",
            );
        }
        if condition == "inspection_gap" {
            watch.reset();
        }
        let processes = match condition {
            "same_pid" => vec![(10, "Claude".into())],
            "multiple" => vec![(20, "Claude".into()), (21, "Claude".into())],
            "unknown" => vec![(20, "Claude --user-data-dir=/unknown".into())],
            _ => vec![(20, "Claude".into())],
        };
        let at = now + Duration::from_secs(if condition == "expired" { 91 } else { 3 });
        let revision = if condition == "unchanged_app" {
            "old"
        } else {
            "new"
        };
        assert!(
            watch.observe(at, &processes, None, revision).is_none(),
            "{condition}"
        );
    }
}

use csw_core::switcher::pending::{DesktopState, LaunchOperations, LaunchPhase, LaunchQueue};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Ops {
    state: AtomicUsize,
    launches: AtomicUsize,
}
impl Ops {
    fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
            launches: AtomicUsize::new(0),
        }
    }
}
impl LaunchOperations for Ops {
    fn validate(&self, _: &str) -> Result<String, &'static str> {
        Ok("identity".into())
    }
    fn inspect(&self, _: &str) -> Result<DesktopState, &'static str> {
        Ok(match self.state.load(Ordering::SeqCst) {
            0 => DesktopState::Running,
            1 => DesktopState::Stopped,
            _ => DesktopState::TargetRunning(30),
        })
    }
    fn launch(&self, _: &str, identity: &str) -> Result<(), &'static str> {
        assert_eq!(identity, "identity");
        assert_eq!(self.state.load(Ordering::SeqCst), 1);
        self.launches.fetch_add(1, Ordering::SeqCst);
        self.state.store(2, Ordering::SeqCst);
        Ok(())
    }
    fn activate(&self, _: u32) -> Result<bool, &'static str> {
        Ok(false)
    }
}

#[test]
fn recovery_uses_the_normal_queue_and_waits_for_exit_before_one_launch() {
    let queue = LaunchQueue::default();
    let ops = Ops::new();
    let quits = AtomicUsize::new(0);
    let request = queue
        .request_prepared("Chris", "identity", &ops, || {
            quits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    assert_eq!(quits.load(Ordering::SeqCst), 1);
    assert!(request.automatic_return);
    assert_eq!(
        queue.advance(request.request_id, &ops).unwrap().phase,
        LaunchPhase::Waiting
    );
    assert_eq!(ops.launches.load(Ordering::SeqCst), 0);
    ops.state.store(1, Ordering::SeqCst);
    queue.advance(request.request_id, &ops).unwrap();
    queue.advance(request.request_id, &ops).unwrap();
    assert_eq!(ops.launches.load(Ordering::SeqCst), 1);
    assert_eq!(queue.status().unwrap().phase, LaunchPhase::Completed);
}

#[test]
fn existing_requests_changed_identity_and_quit_refusal_do_not_start_recovery() {
    for condition in ["busy", "changed", "refused"] {
        let queue = LaunchQueue::default();
        let ops = Ops::new();
        let quits = AtomicUsize::new(0);
        if condition == "busy" {
            queue.request("Other", &ops).unwrap();
        }
        let identity = if condition == "changed" {
            "old identity"
        } else {
            "identity"
        };
        assert!(
            queue
                .request_prepared("Chris", identity, &ops, || {
                    quits.fetch_add(1, Ordering::SeqCst);
                    Err("quit_refused")
                })
                .is_err()
        );
        assert_eq!(
            quits.load(Ordering::SeqCst),
            usize::from(condition == "refused")
        );
        assert_eq!(ops.launches.load(Ordering::SeqCst), 0);
        if condition != "busy" {
            assert_eq!(queue.status().unwrap().phase, LaunchPhase::Idle);
        }
    }
}

#[test]
fn update_return_can_be_canceled_before_exit_without_launching() {
    let queue = LaunchQueue::default();
    let ops = Ops::new();
    let request = queue
        .request_prepared("Chris", "identity", &ops, || Ok(()))
        .unwrap();
    queue.cancel(request.request_id).unwrap();
    ops.state.store(1, Ordering::SeqCst);
    queue.advance(request.request_id, &ops).unwrap();
    assert_eq!(ops.launches.load(Ordering::SeqCst), 0);
}

#[test]
fn preference_is_private_persistent_and_invalid_files_cannot_enable_it() {
    use csw_core::update_return::{enabled, set_enabled};
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("settings");
    assert!(!enabled(&root).unwrap());
    set_enabled(&root, true).unwrap();
    assert!(enabled(&root).unwrap());
    assert_eq!(
        std::fs::metadata(root.join("update-return.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(root.join("update-return.toml"))
            .unwrap()
            .nlink(),
        1
    );
    set_enabled(&root, false).unwrap();
    assert!(!enabled(&root).unwrap());
    std::fs::write(
        root.join("update-return.toml"),
        "enabled = true\nunknown = true",
    )
    .unwrap();
    assert!(enabled(&root).is_err());
    std::fs::remove_file(root.join("update-return.toml")).unwrap();
    let external = dir.path().join("external.toml");
    std::fs::write(&external, "enabled = true").unwrap();
    symlink(&external, root.join("update-return.toml")).unwrap();
    assert!(enabled(&root).is_err());
    assert!(set_enabled(&root, false).is_err());
    assert_eq!(std::fs::read_to_string(external).unwrap(), "enabled = true");
}
