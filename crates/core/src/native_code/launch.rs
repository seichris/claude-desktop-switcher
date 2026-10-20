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

/// Explicitly abandon an unresolved request after every writer has stopped.
pub fn recover_closed_launch(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    abandon: bool,
) -> Result<Option<PathBuf>> {
    if !abandon {
        return Err(refused("explicit abandonment is required"));
    }
    let _guard = lock(provider)?;
    stopped(provider)?;
    let root = provider.app_data_dir();
    let no_journal = || -> Result<()> {
        match fs::symlink_metadata(root.join(JOURNAL)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
            Ok(_) => Err(refused("an interrupted copy must be resolved first")),
        }
    };
    no_journal()?;
    let marker = root.join(LAUNCH);
    match fs::symlink_metadata(&marker) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(_) => (),
    }
    let reservation = read_regular(&marker, &mut { MAX_CONFIG })?;
    let attempt: Attempt = serde_json::from_slice(&reservation)
        .map_err(|_| refused("unsupported launch reservation; choose an explicit source"))?;
    let group = load(provider)?.ok_or_else(|| refused("missing sharing configuration"))?;
    let config = read_regular(&root.join(CONFIG), &mut { MAX_CONFIG })?;
    let places = locations(manager, &group)?;
    let participant = places.iter().find(|p| p.name == attempt.launch.name);
    let profile = manager.get_profile(&attempt.launch.name)?;
    let identity = participant
        .map(Location::identity)
        .unwrap_or_else(|| Identity {
            desktop: profile.isolation.desktop_user_data_dir.clone(),
            cli: profile.isolation.cli_config_dir.clone(),
            bucket: PathBuf::new(),
        });
    if attempt.version != 1
        || group.version != 2
        || attempt.generation != group.generation
        || !valid_tag(&attempt.launch.token)
        || attempt.participating != participant.is_some()
        || attempt.launch.identity != identity
    {
        return Err(refused(
            "the saved launch no longer matches its environment",
        ));
    }
    let stores = bounded_stores(places.iter().map(Store::read), MAX_BYTES)?;
    // Reuse the normal source validation without committing authority. With no
    // prior confirmation, only the attempted participant may have new content.
    authority_index(provider, &group, &places, &stores, None)?;
    for (place, store) in places.iter().zip(&stores) {
        let baseline = group
            .content_fingerprints
            .get(&place.name)
            .ok_or_else(|| refused("missing content baseline"))?;
        if group.last_launch.is_none()
            && place.name != attempt.launch.name
            && *baseline != store.content_fingerprint()?
        {
            return Err(conflict("content changed outside the unconfirmed launch"));
        }
    }
    let backup_root = root.join("native-code-backups");
    safe_fs::private_dir(&backup_root)?;
    let backup = backup_root.join(nonce()?);
    safe_fs::private_dir(&backup)?;
    for (i, store) in stores.iter().enumerate() {
        store.snapshot(&backup.join(i.to_string()))?;
    }
    write_private(&backup.join(CONFIG), &config)?;
    write_private(&backup.join(LAUNCH), &reservation)?;
    write_private(
        &backup.join("locations.json"),
        &serde_json::to_vec(
            &places
                .iter()
                .map(|p| (&p.name, &p.desktop, &p.bucket))
                .collect::<Vec<_>>(),
        )?,
    )?;
    sync_directories(&backup)?;
    sync_directory(&backup_root)?;
    let current_places = locations(manager, &group)?;
    let current_stores = bounded_stores(current_places.iter().map(Store::read), MAX_BYTES)?;
    if load(provider)?.as_ref() != Some(&group)
        || read_regular(&root.join(CONFIG), &mut { MAX_CONFIG })? != config
        || current_places
            .iter()
            .map(Location::identity)
            .collect::<Vec<_>>()
            != places.iter().map(Location::identity).collect::<Vec<_>>()
        || current_stores
            .iter()
            .map(Store::fingerprint)
            .collect::<Vec<_>>()
            != stores.iter().map(Store::fingerprint).collect::<Vec<_>>()
    {
        return Err(refused("sharing state changed while backing up the launch"));
    }
    let current_profile = manager.get_profile(&attempt.launch.name)?;
    if current_profile.isolation.desktop_user_data_dir != attempt.launch.identity.desktop
        || current_profile.isolation.cli_config_dir != attempt.launch.identity.cli
    {
        return Err(refused("the launch environment changed during recovery"));
    }
    stopped(provider)?;
    no_journal()?;
    if read_regular(&marker, &mut { MAX_CONFIG })? != reservation {
        return Err(refused("the launch reservation changed during recovery"));
    }
    // This is abandonment, never a successful-launch attestation. The only
    // live mutation is moving this exact reservation into its durable backup.
    fs::rename(&marker, backup.join("abandoned-launch.json"))?;
    sync_directory(&backup)?;
    sync_directory(&root)?;
    Ok(Some(backup))
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
    if !running || processes.is_empty() {
        return Ok(false);
    }
    if processes
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::mock::MockPlatformProvider;

    #[test]
    fn process_appearing_between_samples_is_retried() {
        let provider = MockPlatformProvider::new(
            "/synthetic/default".into(),
            "/synthetic/cli".into(),
            "/synthetic/switcher".into(),
        );
        let attempt = Attempt {
            version: 1,
            generation: "synthetic".into(),
            participating: true,
            launch: ConfirmedLaunch {
                token: "synthetic".into(),
                name: "second".into(),
                identity: Identity {
                    desktop: "/synthetic/second".into(),
                    bucket: "/synthetic/second/bucket".into(),
                    cli: "/synthetic/cli".into(),
                },
            },
        };
        let process = (42, "Claude --user-data-dir=/synthetic/second".into());
        provider.set_desktop_state(false, vec![process.clone()]);
        assert!(matches!(observed(&provider, &attempt), Ok(false)));
        provider.set_desktop_state(true, vec![]);
        assert!(matches!(observed(&provider, &attempt), Ok(false)));
        provider.set_desktop_state(true, vec![process]);
        assert!(matches!(observed(&provider, &attempt), Ok(true)));
    }
}
