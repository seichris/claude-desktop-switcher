//! GUI-only automatic switching. The CLI's stopped-process guard stays strict.

use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::error::{CswError, Result};
use crate::platform::PlatformProvider;
use crate::profile::ProfileManager;

use super::automatic_flow::{self as flow, Actions, QuitProgress, Situation, Stage};
use super::{ContextSwitcher, environment_for_args_line};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Waiting,
    Switching,
    Completed,
    Failed,
}

impl Phase {
    fn pending(self) -> bool {
        matches!(self, Self::Checking | Self::Waiting | Self::Switching)
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Status {
    pub sequence: u64,
    pub phase: Phase,
    pub target: Option<String>,
    pub error: Option<String>,
}

/// A reservation cannot be forged by the WebView or reused for another target.
pub struct Ticket {
    sequence: u64,
    target: String,
    launch: bool,
}

#[derive(Default)]
pub struct AutomaticSwitcher {
    state: Mutex<Status>,
}

impl AutomaticSwitcher {
    pub fn status(&self) -> Result<Status> {
        self.state
            .lock()
            .map(|s| s.clone())
            .map_err(|_| error("state_unavailable"))
    }

    pub fn begin(&self, manager: &ProfileManager, target: &str, launch: bool) -> Result<Ticket> {
        manager.get_profile(target)?;
        let mut state = self.state.lock().map_err(|_| error("state_unavailable"))?;
        if state.phase.pending() {
            return Err(error("pending"));
        }
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| error("state_unavailable"))?;
        *state = Status {
            sequence,
            phase: Phase::Checking,
            target: Some(target.to_owned()),
            error: None,
        };
        Ok(Ticket {
            sequence,
            target: target.to_owned(),
            launch,
        })
    }

    /// Called once on a blocking worker, never on Tauri's UI thread. Status is
    /// retained so a reloaded settings window can recover an in-flight request.
    pub fn run(
        &self,
        ticket: Ticket,
        manager: Arc<ProfileManager>,
        provider: Arc<dyn PlatformProvider>,
    ) {
        if !self
            .state
            .lock()
            .is_ok_and(|s| s.sequence == ticket.sequence && s.phase == Phase::Checking)
        {
            return;
        }
        let mut runtime = DesktopActions {
            switcher: ContextSwitcher::new(provider.clone(), manager.clone()),
            manager,
            provider: provider.as_ref(),
            target: &ticket.target,
            launch: ticket.launch,
            pid: None,
            desktop: None,
            #[cfg(target_os = "macos")]
            quit: None,
        };
        let result = flow::run(
            &mut runtime,
            |stage| {
                self.update(
                    ticket.sequence,
                    match stage {
                        Stage::Waiting => Phase::Waiting,
                        Stage::Switching => Phase::Switching,
                    },
                    None,
                );
            },
            || std::thread::sleep(std::time::Duration::from_millis(500)),
        );
        match result {
            Ok(()) => self.update(ticket.sequence, Phase::Completed, None),
            Err(e) => self.update(ticket.sequence, Phase::Failed, Some(public_error(&e))),
        }
    }

    fn update(&self, sequence: u64, phase: Phase, error: Option<String>) {
        if let Ok(mut state) = self.state.lock()
            && state.sequence == sequence
        {
            state.phase = phase;
            state.error = error;
        }
    }
}

fn error(code: &str) -> CswError {
    CswError::Other(format!("automatic_{code}"))
}

// Do not send arbitrary filesystem paths or native error descriptions to the UI.
fn public_error(e: &CswError) -> String {
    let message = e.to_string();
    if message.starts_with("automatic_")
        && message.len() <= 90
        && message.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        message
    } else {
        "automatic_failed".to_owned()
    }
}

struct DesktopActions<'a> {
    manager: Arc<ProfileManager>,
    switcher: ContextSwitcher,
    provider: &'a dyn PlatformProvider,
    target: &'a str,
    launch: bool,
    pid: Option<u32>,
    desktop: Option<std::path::PathBuf>,
    #[cfg(target_os = "macos")]
    quit: Option<crate::platform::automatic::SafeQuit<'a>>,
}

impl Actions for DesktopActions<'_> {
    type Error = CswError;
    fn inspect(&mut self) -> Result<Situation> {
        let processes = self.provider.running_desktop_processes()?;
        let running = self.provider.is_claude_desktop_running()?;
        if processes.is_empty() && !running {
            return Ok(Situation::Stopped);
        }
        if processes.len() != 1 || !running {
            return Err(error("multiple_instances"));
        }
        let (pid, args) = &processes[0];
        let envs = self
            .manager
            .list_profiles()?
            .into_iter()
            .map(|n| {
                self.manager
                    .get_profile(&n)
                    .map(|p| (n, p.isolation.desktop_user_data_dir))
            })
            .collect::<Result<Vec<_>>>()?;
        let name =
            environment_for_args_line(args, &envs, &self.provider.claude_desktop_default_dir())
                .ok_or_else(|| error("unmanaged_launch"))?;
        self.pid = Some(*pid);
        if name == self.target {
            return Ok(Situation::TargetRunning);
        }
        self.desktop = Some(
            self.manager
                .get_profile(&name)?
                .isolation
                .desktop_user_data_dir,
        );
        Ok(Situation::Running)
    }

    fn request_quit(&mut self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let mut quit = crate::platform::automatic::SafeQuit::prepare(
                self.provider,
                self.pid.ok_or_else(|| error("identity_changed"))?,
                self.desktop
                    .as_deref()
                    .ok_or_else(|| error("identity_changed"))?,
            )?;
            quit.request()?;
            self.quit = Some(quit);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        Err(error("unsupported_platform"))
    }

    fn poll_quit(&mut self) -> Result<QuitProgress> {
        #[cfg(target_os = "macos")]
        {
            use crate::platform::automatic::QuitProgress as Native;
            self.quit
                .as_mut()
                .ok_or_else(|| error("identity_changed"))?
                .poll()
                .map(|p| match p {
                    Native::Running => QuitProgress::Running,
                    Native::Waiting => QuitProgress::Waiting,
                    Native::Exited => QuitProgress::Exited,
                })
        }
        #[cfg(not(target_os = "macos"))]
        Err(error("unsupported_platform"))
    }

    fn finish(&mut self) -> Result<()> {
        // Fresh live guard, existing durable sync/launch locks, and target lookup
        // deliberately run again after the potentially unbounded native wait.
        self.switcher.switch_to(self.target)?;
        if self.launch {
            let profile = self.manager.get_profile(self.target)?;
            super::desktop::launch_desktop(&profile, self.provider)?;
        }
        Ok(())
    }

    fn activate(&mut self) -> Result<()> {
        if self
            .provider
            .activate_pid(self.pid.ok_or_else(|| error("identity_changed"))?)?
        {
            Ok(())
        } else {
            Err(error("identity_changed"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::mock::MockPlatformProvider;
    #[test]
    fn validates_before_reserving_and_rejects_duplicate_requests() {
        let root = tempfile::tempdir().unwrap();
        let provider = Arc::new(MockPlatformProvider::new(
            root.path().join("desktop"),
            root.path().join("cli"),
            root.path().join("csw"),
        ));
        let manager = ProfileManager::new(provider).unwrap();
        let switcher = AutomaticSwitcher::default();
        assert!(switcher.begin(&manager, "missing", true).is_err());
        assert_eq!(switcher.status().unwrap().phase, Phase::Idle);
        let _ticket = switcher.begin(&manager, "default", true).unwrap();
        assert_eq!(
            switcher
                .begin(&manager, "default", true)
                .err()
                .unwrap()
                .to_string(),
            "automatic_pending"
        );
        assert_eq!(switcher.status().unwrap().phase, Phase::Checking);
    }

    #[test]
    fn ui_errors_never_expose_native_or_filesystem_details() {
        assert_eq!(
            public_error(&error("unknown_dialog")),
            "automatic_unknown_dialog"
        );
        assert_eq!(
            public_error(&CswError::Other("/private/secret".into())),
            "automatic_failed"
        );
        assert_eq!(
            public_error(&CswError::Other("automatic_bad /private".into())),
            "automatic_failed"
        );
    }
}
