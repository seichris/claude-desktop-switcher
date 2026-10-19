use crate::error::Result;
use crate::platform::PlatformProvider;
use crate::profile::Profile;

/// Launch Claude Desktop for the given profile using --user-data-dir
pub fn launch_desktop(
    profile: &Profile,
    provider: &dyn PlatformProvider,
    manager: &crate::profile::ProfileManager,
) -> Result<()> {
    let guard = crate::native_code::launch_guard(manager, provider, profile)?;
    let launched = provider.launch_claude_desktop(
        &profile.isolation.desktop_user_data_dir,
        Some(&profile.isolation.cli_config_dir),
    );
    launched.map_err(|_| crate::error::CswError::DesktopLaunchFailed)?;
    if let Some(guard) = &guard {
        crate::native_code::finish_launch(manager, provider, guard).map_err(|e| {
            if matches!(e, crate::error::CswError::ProcessInspection) {
                crate::error::CswError::LaunchProcessInspection
            } else {
                crate::error::CswError::LaunchUnconfirmed
            }
        })?;
    }

    // Stamp the launch time for the environment list ("last launched"). Every
    // CSW-initiated launch funnels through here, so this is the single point
    // of truth. Two deliberate exceptions:
    // - the default environment: its data dirs are the user's real Claude
    //   directories, and writing a state.toml there would break zero-impact;
    // - a failed stamp: the stamp is informational (profile::state) and must
    //   never turn a successful launch into an error.
    if !profile.profile.is_default
        && let Some(profile_dir) = profile.isolation.desktop_user_data_dir.parent()
    {
        let _ = crate::profile::state::record_last_launch(profile_dir);
    }

    Ok(())
}
