//! Durable launch provenance; requests and active-profile markers are not proof.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    version: u32,
    generation: String,
    participating: bool,
    launch: ConfirmedLaunch,
}

pub struct LaunchGuard {
    _lock: File,
    attempt: Attempt,
    group: Group,
}

pub(super) fn no_pending_launch(provider: &dyn PlatformProvider) -> Result<()> {
    let path = provider.app_data_dir().join(LAUNCH);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
        Ok(_) => (),
    }
    // Legacy unstructured reservations remain blocked. Never infer successful
    // startup from a timeout, process disappearance, or an active marker.
    let bytes = read_regular(&path, &mut { MAX_CONFIG })?;
    if let Ok(attempt) = serde_json::from_slice::<Attempt>(&bytes)
        && attempt.version == 1
        && attempt.participating
        && let Some(group) = load(provider)?
        && group.generation == attempt.generation
        && group.last_launch.as_ref() == Some(&attempt.launch)
    {
        fs::remove_file(path)?;
        return sync_directory(&provider.app_data_dir());
    }
    Err(CswError::LocalCodeLaunchPending)
}

/// A visible process cannot resolve a launch whose provenance was not committed.
/// Only a matching already-committed reservation may be cleaned automatically.
pub fn ensure_confirmed_launch(provider: &dyn PlatformProvider) -> Result<()> {
    match fs::symlink_metadata(provider.app_data_dir().join(LAUNCH)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
        Ok(_) => (),
    }
    let _guard = lock(provider)?;
    no_pending_launch(provider)
}

/// Re-synchronize under the lock held through process observation and commit.
/// This closes the gap between changing the active marker and submitting open.
pub fn launch_guard(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    profile: &Profile,
) -> Result<Option<LaunchGuard>> {
    if load(provider)?.is_none() && !provider.app_data_dir().join(JOURNAL).try_exists()? {
        return Ok(None);
    }
    let guard = lock(provider)?;
    no_pending_launch(provider)?;
    recover(manager, provider)?;
    let Some(mut group) = load(provider)? else {
        return Ok(None);
    };
    let participating =
        group.source == profile.profile.name || group.members.contains(&profile.profile.name);
    let current = manager.get_profile(&profile.profile.name)?;
    if serde_json::to_vec(&current)? != serde_json::to_vec(profile)? {
        return Err(refused("the requested environment changed before launch"));
    }
    let identity = if participating {
        synchronize(manager, provider, &mut group, None)?;
        group
            .identities
            .get(&profile.profile.name)
            .ok_or_else(|| refused("missing launch identity"))?
            .clone()
    } else {
        Identity {
            desktop: profile.isolation.desktop_user_data_dir.clone(),
            cli: profile.isolation.cli_config_dir.clone(),
            bucket: PathBuf::new(),
        }
    };
    let attempt = Attempt {
        version: 1,
        generation: group.generation.clone(),
        participating,
        launch: ConfirmedLaunch {
            token: nonce()?,
            name: profile.profile.name.clone(),
            identity,
        },
    };
    if participating {
        stopped(provider)?;
    }
    write_private(
        &provider.app_data_dir().join(LAUNCH),
        &serde_json::to_vec(&attempt)?,
    )?;
    Ok(Some(LaunchGuard {
        _lock: guard,
        attempt,
        group,
    }))
}

fn observed(provider: &dyn PlatformProvider, attempt: &Attempt) -> Result<bool> {
    let running = provider
        .is_claude_desktop_running()
        .map_err(|_| CswError::ProcessInspection)?;
    let processes = provider
        .running_desktop_processes()
        .map_err(|_| CswError::ProcessInspection)?;
    if !running && processes.is_empty() {
        return Ok(false);
    }
    if !running
        || processes.is_empty()
        || processes
            .iter()
            .any(|(pid, args)| *pid == 0 || args.trim().is_empty())
        || (attempt.participating && processes.len() != 1)
    {
        return Err(CswError::ProcessInspection);
    }
    let envs = [(
        attempt.launch.name.clone(),
        attempt.launch.identity.desktop.clone(),
    )];
    let matches = processes
        .iter()
        .filter(|(_, args)| {
            crate::switcher::environment_for_args_line(
                args,
                &envs,
                &provider.claude_desktop_default_dir(),
            )
            .as_deref()
                == Some(&attempt.launch.name)
        })
        .count();
    if matches != 1 {
        return Err(CswError::LaunchUnconfirmed);
    }
    Ok(true)
}

/// Only an observed exact target can advance authority. Any error keeps the
/// durable reservation, including OS errors that may follow asynchronous open.
pub fn finish_launch(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    guard: &LaunchGuard,
) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !observed(provider, &guard.attempt)? {
        if std::time::Instant::now() >= deadline {
            return Err(CswError::LaunchUnconfirmed);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let profile = manager.get_profile(&guard.attempt.launch.name)?;
    let identity = &guard.attempt.launch.identity;
    if profile.isolation.desktop_user_data_dir != identity.desktop
        || profile.isolation.cli_config_dir != identity.cli
        || (guard.attempt.participating && locate(&profile)?.identity() != *identity)
        || load(provider)?.as_ref() != Some(&guard.group)
    {
        return Err(refused(
            "environment or sharing configuration changed during launch",
        ));
    }
    if guard.attempt.participating {
        let mut group = guard.group.clone();
        group.last_launch = Some(guard.attempt.launch.clone());
        atomic_write(
            &provider.app_data_dir().join(CONFIG),
            toml::to_string(&group)?.as_bytes(),
        )?;
    }
    fs::remove_file(provider.app_data_dir().join(LAUNCH))?;
    sync_directory(&provider.app_data_dir())
}
