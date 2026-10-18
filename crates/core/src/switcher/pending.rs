//! Volatile launch requests. This module has no operation for quitting Claude.
use std::sync::Mutex;

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPhase {
    Idle,
    Waiting,
    Switching,
    Completed,
    Canceled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PendingLaunch {
    pub request_id: u64,
    pub target: Option<String>,
    pub targets: Vec<String>,
    pub phase: LaunchPhase,
    pub error: Option<&'static str>,
    pub wait_for_exit: bool,
}

pub enum DesktopState {
    Stopped,
    Running,
    TargetRunning(u32),
}

/// Implementations must revalidate the stopped condition inside `launch`, using
/// the normal switch/synchronization and launch-reservation path.
pub trait LaunchOperations {
    fn validate(&self, target: &str) -> Result<String, &'static str>;
    fn validate_alongside(&self, _target: &str) -> Result<String, &'static str> {
        Err("alongside_refused")
    }
    fn inspect(&self, target: &str) -> Result<DesktopState, &'static str>;
    fn launch(&self, target: &str, identity: &str) -> Result<(), &'static str>;
    fn launch_alongside(&self, _target: &str, _identity: &str) -> Result<(), &'static str> {
        Err("alongside_refused")
    }
    fn activate(&self, pid: u32) -> Result<bool, &'static str>;
}

struct QueueState {
    status: PendingLaunch,
    identity: String,
    submitted: Option<std::time::Instant>,
    committed_failure: bool,
    alongside: bool,
    batch: bool,
}

pub struct LaunchQueue {
    execution: Mutex<()>,
    state: Mutex<QueueState>,
}

pub struct IdleLaunchReservation<'a> {
    queue: &'a LaunchQueue,
    _execution: std::sync::MutexGuard<'a, ()>,
}

impl IdleLaunchReservation<'_> {
    pub fn observe_batch(&self, targets: Vec<String>) -> Result<PendingLaunch, &'static str> {
        self.queue.observe_batch(targets)
    }
}

impl Default for LaunchQueue {
    fn default() -> Self {
        Self {
            execution: Mutex::new(()),
            state: Mutex::new(QueueState {
                status: PendingLaunch {
                    request_id: 0,
                    target: None,
                    targets: Vec::new(),
                    phase: LaunchPhase::Idle,
                    error: None,
                    wait_for_exit: true,
                },
                identity: String::new(),
                submitted: None,
                committed_failure: false,
                alongside: false,
                batch: false,
            }),
        }
    }
}

impl LaunchQueue {
    pub fn status(&self) -> Result<PendingLaunch, &'static str> {
        self.state
            .lock()
            .map(|s| s.status.clone())
            .map_err(|_| "launch_unavailable")
    }

    pub fn is_pending(&self) -> Result<bool, &'static str> {
        let s = self.state.lock().map_err(|_| "launch_unavailable")?;
        Ok(matches!(
            s.status.phase,
            LaunchPhase::Waiting | LaunchPhase::Switching
        ) || s.committed_failure)
    }

    /// Reserve a legacy explicit batch action against queue admission/commit.
    /// No process state is inferred by this reservation; existing launch guards
    /// still apply to every member of the batch.
    pub fn reserve_idle(&self) -> Result<IdleLaunchReservation<'_>, &'static str> {
        let execution = self.execution.try_lock().map_err(|_| "launch_busy")?;
        if self.is_pending()? {
            return Err("launch_pending");
        }
        Ok(IdleLaunchReservation {
            queue: self,
            _execution: execution,
        })
    }

    /// Called under `reserve_idle` after a legacy batch has submitted its
    /// requests. Keep all of those startups reserved after its stack guard ends.
    fn observe_batch(&self, targets: Vec<String>) -> Result<PendingLaunch, &'static str> {
        if targets.is_empty() {
            return self.status();
        }
        let mut s = self.state.lock().map_err(|_| "launch_unavailable")?;
        if matches!(
            s.status.phase,
            LaunchPhase::Waiting | LaunchPhase::Switching
        ) || s.committed_failure
        {
            return Err("launch_pending");
        }
        let request_id = s
            .status
            .request_id
            .checked_add(1)
            .ok_or("launch_unavailable")?;
        s.status = PendingLaunch {
            request_id,
            target: targets.first().cloned(),
            targets,
            phase: LaunchPhase::Switching,
            error: None,
            wait_for_exit: false,
        };
        s.submitted = Some(std::time::Instant::now());
        s.committed_failure = false;
        s.batch = true;
        s.alongside = false;
        Ok(s.status.clone())
    }

    pub fn request(
        &self,
        target: &str,
        ops: &dyn LaunchOperations,
    ) -> Result<PendingLaunch, &'static str> {
        self.request_mode(target, ops, false)
    }

    pub fn request_alongside(
        &self,
        target: &str,
        ops: &dyn LaunchOperations,
    ) -> Result<PendingLaunch, &'static str> {
        self.request_mode(target, ops, true)
    }

    fn request_mode(
        &self,
        target: &str,
        ops: &dyn LaunchOperations,
        alongside: bool,
    ) -> Result<PendingLaunch, &'static str> {
        {
            let s = self.state.lock().map_err(|_| "launch_unavailable")?;
            if matches!(
                s.status.phase,
                LaunchPhase::Waiting | LaunchPhase::Switching
            ) || s.committed_failure
            {
                return if s.status.target.as_deref() == Some(target) && s.alongside == alongside {
                    Ok(s.status.clone())
                } else {
                    Err("launch_pending")
                };
            }
        }
        let _execution = self.execution.try_lock().map_err(|_| "launch_busy")?;
        let identity = if alongside {
            ops.validate_alongside(target)?
        } else {
            ops.validate(target)?
        };
        let mut s = self.state.lock().map_err(|_| "launch_unavailable")?;
        if matches!(
            s.status.phase,
            LaunchPhase::Waiting | LaunchPhase::Switching
        ) || s.committed_failure
        {
            if s.status.target.as_deref() == Some(target) && s.alongside == alongside {
                return Ok(s.status.clone());
            }
            return Err("launch_pending");
        }
        let request_id = s
            .status
            .request_id
            .checked_add(1)
            .ok_or("launch_unavailable")?;
        s.status = PendingLaunch {
            request_id,
            target: Some(target.into()),
            targets: vec![target.into()],
            phase: LaunchPhase::Waiting,
            error: None,
            wait_for_exit: !alongside,
        };
        s.identity = identity;
        s.submitted = None;
        s.committed_failure = false;
        s.alongside = alongside;
        s.batch = false;
        Ok(s.status.clone())
    }

    pub fn cancel(&self, request_id: u64) -> Result<PendingLaunch, &'static str> {
        let mut s = self.state.lock().map_err(|_| "launch_unavailable")?;
        if s.status.request_id != request_id {
            return Err("launch_stale");
        }
        if s.status.phase != LaunchPhase::Waiting {
            return Err("launch_committed");
        }
        s.status.phase = LaunchPhase::Canceled;
        Ok(s.status.clone())
    }

    /// Inspect outside the state lock so cancellation stays responsive. The
    /// generation and Waiting -> Switching transition are rechecked under the
    /// same lock before any effects. Only that winner can synchronize or launch.
    pub fn advance(
        &self,
        request_id: u64,
        ops: &dyn LaunchOperations,
    ) -> Result<PendingLaunch, &'static str> {
        let (target, identity, observing, alongside, batch_targets) = {
            let s = self.state.lock().map_err(|_| "launch_unavailable")?;
            if s.status.request_id != request_id {
                return Ok(s.status.clone());
            }
            match s.status.phase {
                LaunchPhase::Waiting => {}
                LaunchPhase::Switching | LaunchPhase::Failed if s.submitted.is_some() => {}
                _ => return Ok(s.status.clone()),
            }
            (
                s.status.target.clone().ok_or("launch_unavailable")?,
                s.identity.clone(),
                s.submitted.is_some(),
                s.alongside,
                if s.batch {
                    s.status.targets.clone()
                } else {
                    Vec::new()
                },
            )
        };
        let inspection = if batch_targets.is_empty() {
            ops.inspect(&target)
        } else {
            batch_targets
                .iter()
                .try_fold(DesktopState::TargetRunning(0), |acc, target| {
                    let next = ops.inspect(target)?;
                    Ok(
                        if matches!(acc, DesktopState::TargetRunning(_))
                            && matches!(next, DesktopState::TargetRunning(_))
                        {
                            DesktopState::TargetRunning(0)
                        } else {
                            DesktopState::Running
                        },
                    )
                })
        };
        let validation = if observing {
            Ok(identity.clone())
        } else if alongside {
            ops.validate_alongside(&target)
        } else {
            ops.validate(&target)
        };
        let _execution = match self.execution.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return self.status(),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err("launch_unavailable"),
        };
        let mut s = self.state.lock().map_err(|_| "launch_unavailable")?;
        if s.status.request_id != request_id
            || (!observing && s.status.phase != LaunchPhase::Waiting)
            || (observing
                && !matches!(s.status.phase, LaunchPhase::Switching | LaunchPhase::Failed))
        {
            return Ok(s.status.clone());
        }
        if observing {
            match inspection {
                Ok(DesktopState::TargetRunning(_)) => {
                    s.status.phase = LaunchPhase::Completed;
                    s.status.error = None;
                    s.committed_failure = false;
                }
                Err(error) => s.status.error = Some(error),
                _ if s
                    .submitted
                    .is_some_and(|t| t.elapsed() >= std::time::Duration::from_secs(10)) =>
                {
                    s.status.phase = LaunchPhase::Failed;
                    s.status.error = Some("launch_unconfirmed");
                    s.committed_failure = true;
                }
                _ => {}
            }
            return Ok(s.status.clone());
        }
        let error = match validation {
            Ok(current) if current == identity => inspection.as_ref().err().copied(),
            Ok(_) => Some("target_changed"),
            Err(error) => Some(error),
        };
        if let Some(error) = error {
            s.status.phase = LaunchPhase::Failed;
            s.status.error = Some(error);
            return Ok(s.status.clone());
        }
        let desktop = inspection.map_err(|_| "process_unavailable")?;
        if matches!(desktop, DesktopState::Running) && !alongside {
            return Ok(s.status.clone());
        }
        s.status.phase = LaunchPhase::Switching;
        drop(s);
        // No cancellation/replacement can succeed beyond the committed boundary.
        let outcome = match desktop {
            DesktopState::TargetRunning(pid) => ops.activate(pid).and_then(|found| {
                if found {
                    Ok(false)
                } else {
                    Err("process_unavailable")
                }
            }),
            DesktopState::Stopped | DesktopState::Running if alongside => {
                ops.launch_alongside(&target, &identity).map(|()| true)
            }
            DesktopState::Stopped => ops.launch(&target, &identity).map(|()| true),
            DesktopState::Running => return Err("launch_unavailable"),
        };
        let mut s = self.state.lock().map_err(|_| "launch_unavailable")?;
        match outcome {
            Ok(true) => {
                s.submitted = Some(std::time::Instant::now());
            }
            Ok(false) => s.status.phase = LaunchPhase::Completed,
            Err(error) => {
                s.status.phase = LaunchPhase::Failed;
                s.status.error = Some(error);
                // A launch error can occur after LaunchServices accepted it.
                // Never repeat an uncertain launch, even if it isn't visible yet.
                if error == "launch_unconfirmed" {
                    s.submitted = Some(std::time::Instant::now());
                    s.committed_failure = true;
                }
            }
        }
        Ok(s.status.clone())
    }
}

/// Production effects are deliberately limited to existing profile metadata,
/// process inspection, activation and the existing guarded launch path.
pub struct DesktopLaunchOperations {
    provider: std::sync::Arc<dyn crate::platform::PlatformProvider>,
    manager: std::sync::Arc<crate::profile::ProfileManager>,
}

impl DesktopLaunchOperations {
    pub fn new(
        provider: std::sync::Arc<dyn crate::platform::PlatformProvider>,
        manager: std::sync::Arc<crate::profile::ProfileManager>,
    ) -> Self {
        Self { provider, manager }
    }
}

impl LaunchOperations for DesktopLaunchOperations {
    fn validate(&self, target: &str) -> Result<String, &'static str> {
        let profile = self
            .manager
            .get_profile(target)
            .map_err(|_| "target_unavailable")?;
        // Only CSW's profile metadata; never authentication files or stores.
        serde_json::to_string(&profile).map_err(|_| "target_unavailable")
    }
    fn validate_alongside(&self, target: &str) -> Result<String, &'static str> {
        let profile = self
            .manager
            .get_profile(target)
            .map_err(|_| "target_unavailable")?;
        if profile.profile.is_default || !profile.sharing.is_fully_isolated() {
            return Err("alongside_refused");
        }
        self.validate(target)
    }
    fn inspect(&self, target: &str) -> Result<DesktopState, &'static str> {
        let profile = self
            .manager
            .get_profile(target)
            .map_err(|_| "target_unavailable")?;
        let processes = self
            .provider
            .running_desktop_processes()
            .map_err(|_| "process_unavailable")?;
        let running = self
            .provider
            .is_claude_desktop_running()
            .map_err(|_| "process_unavailable")?;
        if running == processes.is_empty() {
            return Err("process_unavailable");
        }
        let mut seen = std::collections::HashSet::new();
        if processes
            .iter()
            .any(|(pid, args)| *pid == 0 || args.trim().is_empty() || !seen.insert(*pid))
        {
            return Err("process_unavailable");
        }
        let default = self.provider.claude_desktop_default_dir();
        let envs = [(target.into(), profile.isolation.desktop_user_data_dir)];
        let matches: Vec<_> = processes
            .iter()
            .filter(|(_, args)| {
                super::environment_for_args_line(args, &envs, &default).as_deref() == Some(target)
            })
            .collect();
        if matches.len() > 1 {
            return Err("process_unavailable");
        }
        if let Some((pid, _)) = matches.first() {
            return Ok(DesktopState::TargetRunning(*pid));
        }
        Ok(if running {
            DesktopState::Running
        } else {
            DesktopState::Stopped
        })
    }
    fn launch(&self, target: &str, identity: &str) -> Result<(), &'static str> {
        if self.validate(target)? != identity {
            return Err("target_changed");
        }
        if !matches!(self.inspect(target)?, DesktopState::Stopped) {
            return Err("desktop_reappeared");
        }
        super::ContextSwitcher::new(self.provider.clone(), self.manager.clone())
            .switch_to(target)
            .map_err(|_| "launch_refused")?;
        let profile = self
            .manager
            .get_profile(target)
            .map_err(|_| "target_unavailable")?;
        if !matches!(self.inspect(target)?, DesktopState::Stopped) {
            return Err("desktop_reappeared");
        }
        if self.validate(target)? != identity {
            return Err("target_changed");
        }
        super::desktop::launch_desktop(&profile, self.provider.as_ref())
            .map_err(|_| "launch_unconfirmed")
    }
    fn launch_alongside(&self, target: &str, identity: &str) -> Result<(), &'static str> {
        self.validate_alongside(target)?;
        if matches!(self.inspect(target)?, DesktopState::TargetRunning(_)) {
            return Err("desktop_reappeared");
        }
        let profile = self
            .manager
            .get_profile(target)
            .map_err(|_| "target_unavailable")?;
        if self.validate(target)? != identity {
            return Err("target_changed");
        }
        super::desktop::launch_desktop(&profile, self.provider.as_ref())
            .map_err(|_| "launch_unconfirmed")
    }
    fn activate(&self, pid: u32) -> Result<bool, &'static str> {
        self.provider
            .activate_pid(pid)
            .map_err(|_| "process_unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Fake {
        running: AtomicUsize,
        target: AtomicBool,
        fail: AtomicBool,
        launches: AtomicUsize,
        activations: AtomicUsize,
        revision: AtomicUsize,
        reappear: AtomicBool,
    }
    impl Fake {
        fn new(running: usize) -> Self {
            Self {
                running: AtomicUsize::new(running),
                target: AtomicBool::new(false),
                fail: AtomicBool::new(false),
                launches: AtomicUsize::new(0),
                activations: AtomicUsize::new(0),
                revision: AtomicUsize::new(0),
                reappear: AtomicBool::new(false),
            }
        }
    }
    impl LaunchOperations for Fake {
        fn validate(&self, target: &str) -> Result<String, &'static str> {
            if target == "Gone" {
                Err("target_unavailable")
            } else {
                Ok(self.revision.load(Ordering::SeqCst).to_string())
            }
        }
        fn inspect(&self, _: &str) -> Result<DesktopState, &'static str> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("process_unavailable");
            }
            if self.target.load(Ordering::SeqCst) {
                Ok(DesktopState::TargetRunning(7))
            } else if self.running.load(Ordering::SeqCst) > 0 {
                Ok(DesktopState::Running)
            } else {
                Ok(DesktopState::Stopped)
            }
        }
        fn launch(&self, _: &str, _: &str) -> Result<(), &'static str> {
            if self.reappear.load(Ordering::SeqCst) {
                return Err("desktop_reappeared");
            }
            self.launches.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn activate(&self, _: u32) -> Result<bool, &'static str> {
            self.activations.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        }
    }

    #[test]
    fn waits_for_every_process_then_launches_once_and_observes_startup() {
        let q = LaunchQueue::default();
        let f = Fake::new(2);
        let r = q.request("Work", &f).unwrap();
        for n in [2, 1] {
            f.running.store(n, Ordering::SeqCst);
            assert_eq!(
                q.advance(r.request_id, &f).unwrap().phase,
                LaunchPhase::Waiting
            );
        }
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
        f.running.store(0, Ordering::SeqCst);
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Switching
        );
        for _ in 0..3 {
            q.advance(r.request_id, &f).unwrap();
        }
        assert_eq!(f.launches.load(Ordering::SeqCst), 1);
        assert_eq!(q.cancel(r.request_id), Err("launch_committed"));
        assert_eq!(q.request("Other", &f), Err("launch_pending"));
        f.target.store(true, Ordering::SeqCst);
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Completed
        );
        assert_eq!(f.activations.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn cancellation_and_stale_tickets_never_launch() {
        let q = LaunchQueue::default();
        let f = Fake::new(1);
        let a = q.request("Work", &f).unwrap();
        assert_eq!(q.request("Work", &f).unwrap(), a);
        assert_eq!(q.request("Other", &f), Err("launch_pending"));
        assert_eq!(q.cancel(a.request_id).unwrap().phase, LaunchPhase::Canceled);
        let b = q.request("Other", &f).unwrap();
        f.running.store(0, Ordering::SeqCst);
        q.advance(a.request_id, &f).unwrap();
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
        assert_eq!(q.cancel(a.request_id), Err("launch_stale"));
        q.cancel(b.request_id).unwrap();
        q.advance(b.request_id, &f).unwrap();
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn inspection_failure_and_deleted_target_fail_closed() {
        let q = LaunchQueue::default();
        let f = Fake::new(0);
        assert_eq!(q.request("Gone", &f), Err("target_unavailable"));
        let r = q.request("Work", &f).unwrap();
        f.fail.store(true, Ordering::SeqCst);
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Failed
        );
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn changed_target_and_process_reappearance_refuse_launch() {
        let q = LaunchQueue::default();
        let f = Fake::new(1);
        let r = q.request("Work", &f).unwrap();
        f.revision.store(1, Ordering::SeqCst);
        f.running.store(0, Ordering::SeqCst);
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().error,
            Some("target_changed")
        );
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
        let r = q.request("Other", &f).unwrap();
        f.reappear.store(true, Ordering::SeqCst);
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().error,
            Some("desktop_reappeared")
        );
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn production_effects_validate_profiles_and_preserve_running_guard() {
        use crate::platform::mock::MockPlatformProvider;
        use crate::profile::{ProfileManager, SharingConfig};
        let temp = tempfile::tempdir().unwrap();
        let p = std::sync::Arc::new(MockPlatformProvider::new(
            temp.path().join("desktop"),
            temp.path().join("cli"),
            temp.path().join("app"),
        ));
        let manager = std::sync::Arc::new(ProfileManager::new(p.clone()).unwrap());
        manager
            .create_profile("Work", SharingConfig::default(), None)
            .unwrap();
        let ops = DesktopLaunchOperations::new(p.clone(), manager.clone());
        let identity = ops.validate("Work").unwrap();
        p.set_desktop_state(
            true,
            vec![
                (10, "...MacOS/Claude".into()),
                (11, "...MacOS/Claude --user-data-dir=/unknown".into()),
            ],
        );
        assert!(matches!(ops.inspect("Work"), Ok(DesktopState::Running)));
        assert_eq!(ops.launch("Work", &identity), Err("desktop_reappeared"));
        assert_eq!(manager.active_profile_name(), "default");
        assert!(p.launched_dirs().is_empty());
        p.set_desktop_state(false, vec![(10, "...MacOS/Claude".into())]);
        assert!(matches!(ops.inspect("Work"), Err("process_unavailable")));
        p.set_desktop_state(false, vec![]);
        let q = LaunchQueue::default();
        let r = q.request("Work", &ops).unwrap();
        assert_eq!(
            q.advance(r.request_id, &ops).unwrap().phase,
            LaunchPhase::Switching
        );
        assert_eq!(manager.active_profile_name(), "Work");
        assert_eq!(p.launched_dirs().len(), 1);
        q.advance(r.request_id, &ops).unwrap();
        assert_eq!(p.launched_dirs().len(), 1);
        let profile = manager.get_profile("Work").unwrap();
        p.set_desktop_state(
            true,
            vec![(
                12,
                format!(
                    "...MacOS/Claude --user-data-dir={}",
                    profile.isolation.desktop_user_data_dir.display()
                ),
            )],
        );
        assert_eq!(
            q.advance(r.request_id, &ops).unwrap().phase,
            LaunchPhase::Completed
        );
        let r = q.request("Work", &ops).unwrap();
        assert_eq!(
            q.advance(r.request_id, &ops).unwrap().phase,
            LaunchPhase::Completed
        );
        assert_eq!(p.activated_pids(), vec![12]);
        assert_eq!(p.launched_dirs().len(), 1);
        assert!(ops.validate_alongside("default").is_err());
        assert!(ops.validate_alongside("Work").is_ok());
        manager
            .create_profile("Shared", SharingConfig::share_settings_preset(), None)
            .unwrap();
        assert!(ops.validate_alongside("Shared").is_err());
        manager
            .create_profile("Isolated", SharingConfig::default(), None)
            .unwrap();
        let r = q.request_alongside("Isolated", &ops).unwrap();
        assert!(!r.wait_for_exit);
        assert_eq!(
            q.advance(r.request_id, &ops).unwrap().phase,
            LaunchPhase::Switching
        );
        assert_eq!(p.launched_dirs().len(), 2);
        assert_eq!(manager.active_profile_name(), "Work");
    }

    #[test]
    fn actual_profile_edit_and_delete_are_rejected_before_effects() {
        use crate::platform::mock::MockPlatformProvider;
        use crate::profile::{ProfileManager, SharingConfig};
        let temp = tempfile::tempdir().unwrap();
        let p = std::sync::Arc::new(MockPlatformProvider::new(
            temp.path().join("desktop"),
            temp.path().join("cli"),
            temp.path().join("app"),
        ));
        let manager = std::sync::Arc::new(ProfileManager::new(p.clone()).unwrap());
        manager
            .create_profile("Work", SharingConfig::default(), None)
            .unwrap();
        let ops = DesktopLaunchOperations::new(p.clone(), manager.clone());
        let q = LaunchQueue::default();
        let a = q.request("Work", &ops).unwrap();
        manager.set_profile_note("Work", "Changed").unwrap();
        assert_eq!(
            q.advance(a.request_id, &ops).unwrap().error,
            Some("target_changed")
        );
        let b = q.request("Work", &ops).unwrap();
        manager.delete_profile("Work").unwrap();
        assert_eq!(
            q.advance(b.request_id, &ops).unwrap().error,
            Some("target_unavailable")
        );
        assert!(p.launched_dirs().is_empty());
        assert_eq!(manager.active_profile_name(), "default");
    }

    #[test]
    fn canceled_inspection_cannot_commit_even_after_replacement() {
        struct Paused {
            fake: Fake,
            entered: std::sync::Barrier,
            resume: std::sync::Barrier,
        }
        impl LaunchOperations for Paused {
            fn validate(&self, t: &str) -> Result<String, &'static str> {
                self.fake.validate(t)
            }
            fn inspect(&self, _: &str) -> Result<DesktopState, &'static str> {
                self.entered.wait();
                self.resume.wait();
                Ok(DesktopState::Stopped)
            }
            fn launch(&self, t: &str, i: &str) -> Result<(), &'static str> {
                self.fake.launch(t, i)
            }
            fn activate(&self, p: u32) -> Result<bool, &'static str> {
                self.fake.activate(p)
            }
        }
        let q = std::sync::Arc::new(LaunchQueue::default());
        let f = std::sync::Arc::new(Paused {
            fake: Fake::new(0),
            entered: std::sync::Barrier::new(2),
            resume: std::sync::Barrier::new(2),
        });
        let first = q.request("Work", f.as_ref()).unwrap();
        let worker = {
            let q = q.clone();
            let f = f.clone();
            std::thread::spawn(move || q.advance(first.request_id, f.as_ref()).unwrap())
        };
        f.entered.wait();
        q.cancel(first.request_id).unwrap();
        let second = q.request("Other", f.as_ref()).unwrap();
        f.resume.wait();
        assert_eq!(worker.join().unwrap().request_id, second.request_id);
        assert_eq!(f.fake.launches.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stale_observer_cannot_regress_completed_startup() {
        struct Slow {
            fake: Fake,
            pause: AtomicBool,
            entered: std::sync::Barrier,
            resume: std::sync::Barrier,
        }
        impl LaunchOperations for Slow {
            fn validate(&self, t: &str) -> Result<String, &'static str> {
                self.fake.validate(t)
            }
            fn inspect(&self, t: &str) -> Result<DesktopState, &'static str> {
                let snapshot = self.fake.inspect(t);
                if self.pause.swap(false, Ordering::SeqCst) {
                    self.entered.wait();
                    self.resume.wait();
                }
                snapshot
            }
            fn launch(&self, t: &str, i: &str) -> Result<(), &'static str> {
                self.fake.launch(t, i)
            }
            fn activate(&self, p: u32) -> Result<bool, &'static str> {
                self.fake.activate(p)
            }
        }
        let q = std::sync::Arc::new(LaunchQueue::default());
        let f = std::sync::Arc::new(Slow {
            fake: Fake::new(0),
            pause: AtomicBool::new(false),
            entered: std::sync::Barrier::new(2),
            resume: std::sync::Barrier::new(2),
        });
        let r = q.request("Work", f.as_ref()).unwrap();
        q.advance(r.request_id, f.as_ref()).unwrap();
        q.state.lock().unwrap().submitted =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(11));
        f.pause.store(true, Ordering::SeqCst);
        let worker = {
            let q = q.clone();
            let f = f.clone();
            std::thread::spawn(move || q.advance(r.request_id, f.as_ref()).unwrap())
        };
        f.entered.wait();
        f.fake.target.store(true, Ordering::SeqCst);
        assert_eq!(
            q.advance(r.request_id, f.as_ref()).unwrap().phase,
            LaunchPhase::Completed
        );
        f.resume.wait();
        assert_eq!(worker.join().unwrap().phase, LaunchPhase::Completed);
        assert!(!q.is_pending().unwrap());
    }

    #[test]
    fn concurrent_workers_submit_only_one_launch() {
        let q = std::sync::Arc::new(LaunchQueue::default());
        let f = std::sync::Arc::new(Fake::new(0));
        let r = q.request("Work", f.as_ref()).unwrap();
        let workers: Vec<_> = (0..12)
            .map(|_| {
                let q = q.clone();
                let f = f.clone();
                std::thread::spawn(move || q.advance(r.request_id, f.as_ref()).unwrap())
            })
            .collect();
        for w in workers {
            w.join().unwrap();
        }
        assert_eq!(f.launches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unconfirmed_startup_is_reserved_without_any_retry() {
        let q = LaunchQueue::default();
        let f = Fake::new(0);
        let r = q.request("Work", &f).unwrap();
        q.advance(r.request_id, &f).unwrap();
        q.state.lock().unwrap().submitted =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(11));
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().error,
            Some("launch_unconfirmed")
        );
        assert!(q.is_pending().unwrap());
        assert_eq!(q.request("Other", &f), Err("launch_pending"));
        assert_eq!(q.cancel(r.request_id), Err("launch_committed"));
        q.advance(r.request_id, &f).unwrap();
        assert_eq!(f.launches.load(Ordering::SeqCst), 1);
        f.target.store(true, Ordering::SeqCst);
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Completed
        );
    }

    #[test]
    fn legacy_batch_keeps_every_startup_reserved_without_submitting_again() {
        struct Batch {
            visible: Mutex<Vec<String>>,
        }
        impl LaunchOperations for Batch {
            fn validate(&self, t: &str) -> Result<String, &'static str> {
                Ok(t.into())
            }
            fn inspect(&self, t: &str) -> Result<DesktopState, &'static str> {
                Ok(if self.visible.lock().unwrap().iter().any(|n| n == t) {
                    DesktopState::TargetRunning(9)
                } else {
                    DesktopState::Stopped
                })
            }
            fn launch(&self, _: &str, _: &str) -> Result<(), &'static str> {
                panic!("batch observation must never launch")
            }
            fn activate(&self, _: u32) -> Result<bool, &'static str> {
                panic!("batch observation must never activate")
            }
        }
        let q = LaunchQueue::default();
        let f = Batch {
            visible: Mutex::new(vec![]),
        };
        let reservation = q.reserve_idle().unwrap();
        let r = reservation
            .observe_batch(vec!["Work".into(), "Research".into()])
            .unwrap();
        drop(reservation);
        assert_eq!(q.request("Other", &f), Err("launch_pending"));
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Switching
        );
        f.visible.lock().unwrap().push("Work".into());
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Switching
        );
        f.visible.lock().unwrap().push("Research".into());
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Completed
        );
    }

    #[test]
    fn legacy_batch_reservation_cannot_race_queue_admission() {
        let q = LaunchQueue::default();
        let f = Fake::new(0);
        let reservation = q.reserve_idle().unwrap();
        assert_eq!(q.request("Work", &f), Err("launch_busy"));
        drop(reservation);
        let r = q.request("Work", &f).unwrap();
        assert!(matches!(q.reserve_idle(), Err("launch_pending")));
        q.cancel(r.request_id).unwrap();
        assert!(q.reserve_idle().is_ok());
    }

    #[test]
    fn existing_target_is_activated_without_switch_or_duplicate_launch() {
        let q = LaunchQueue::default();
        let f = Fake::new(2);
        f.target.store(true, Ordering::SeqCst);
        let r = q.request("Work", &f).unwrap();
        assert_eq!(
            q.advance(r.request_id, &f).unwrap().phase,
            LaunchPhase::Completed
        );
        assert_eq!(f.launches.load(Ordering::SeqCst), 0);
        assert_eq!(f.activations.load(Ordering::SeqCst), 1);
    }
}
