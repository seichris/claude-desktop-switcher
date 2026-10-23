//! Opt-in recovery of a named environment after Claude's updater relaunches
//! the new app without its environment arguments. No task contents are read.
use crate::{
    error::{CswError, Result},
    profile::safe_fs,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

const SETTINGS: &str = "update-return.toml";

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    enabled: bool,
}

pub fn enabled(root: &Path) -> Result<bool> {
    safe_fs::reject_symlink(root)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = match options.open(root.join(SETTINGS)) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > 4096 {
        return Err(CswError::ProcessInspection);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(CswError::ProcessInspection);
        }
    }
    let mut data = String::new();
    file.by_ref().take(4097).read_to_string(&mut data)?;
    if data.len() > 4096 {
        return Err(CswError::ProcessInspection);
    }
    Ok(toml::from_str::<Settings>(&data)?.enabled)
}

pub fn set_enabled(root: &Path, enabled: bool) -> Result<()> {
    use std::io::Write;
    safe_fs::private_dir(root)?;
    safe_fs::reject_symlink(&root.join(SETTINGS))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| CswError::ProcessInspection)?
        .as_nanos();
    let tmp = root.join(format!(".update-return-{}-{nonce}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(toml::to_string(&Settings { enabled })?.as_bytes())?;
    file.sync_all()?;
    fs::rename(tmp, root.join(SETTINGS))?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnPlan {
    pub target: String,
    pub identity: String,
    pub pid: u32,
    pub revision: String,
}

struct Observed {
    target: String,
    identity: String,
    pid: u32,
    revision: String,
    at: Instant,
}

#[derive(Default)]
pub struct UpdateReturnWatch {
    observed: Option<Observed>,
}

impl UpdateReturnWatch {
    pub fn reset(&mut self) {
        self.observed = None;
    }

    /// `named` is resolved from the sole verified live process, never the
    /// selected row. A failed inspection must call reset rather than supply an
    /// empty process list. Consume every candidate so failures never loop.
    pub fn observe(
        &mut self,
        now: Instant,
        processes: &[(u32, String)],
        named: Option<(&str, &str)>,
        revision: &str,
    ) -> Option<ReturnPlan> {
        if let ([(pid, args)], Some((target, identity))) = (processes, named)
            && *pid > 0
            && target != "default"
            && args.contains("--user-data-dir=")
        {
            self.observed = Some(Observed {
                target: target.into(),
                identity: identity.into(),
                pid: *pid,
                revision: revision.into(),
                at: now,
            });
            return None;
        }
        if processes.is_empty() {
            if self
                .observed
                .as_ref()
                .is_some_and(|s| now.duration_since(s.at) > Duration::from_secs(90))
            {
                self.reset();
            }
            return None;
        }
        let prior = self.observed.take()?;
        let [(pid, args)] = processes else {
            return None;
        };
        if *pid == 0
            || *pid == prior.pid
            || args.contains("--user-data-dir")
            || revision == prior.revision
            || now.duration_since(prior.at) > Duration::from_secs(90)
        {
            return None;
        }
        Some(ReturnPlan {
            target: prior.target,
            identity: prior.identity,
            pid: *pid,
            revision: revision.into(),
        })
    }
}
