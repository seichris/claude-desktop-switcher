//! Opt-in, stopped-Desktop synchronization of complete local Code stores.
//! Authentication storage and Claude's executable are never opened here.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    error::{CswError, Result},
    platform::PlatformProvider,
    profile::{Profile, ProfileManager, safe_fs},
};

const CONFIG: &str = "native-code.toml";
const JOURNAL: &str = "native-code-journal.json";
const LAUNCH: &str = "native-code-launch-pending";
const CATALOG: &str = "claude-code-sessions";
const WORKTREES: &str = "git-worktrees.json";
const PRESETS: &str = "ccd-session-presets.json";
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

fn refused(message: &str) -> CswError {
    CswError::Other(format!("Local Code sharing: {message}"))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    version: u32,
    #[serde(default)]
    generation: String,
    source: String,
    members: Vec<String>,
    fingerprints: BTreeMap<String, String>,
}

struct Location {
    name: String,
    desktop: PathBuf,
    bucket: PathBuf,
    account: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    tag: String,
    authority: String,
    group: Group,
    places: Vec<(String, PathBuf, PathBuf)>,
    prior_config: Option<String>,
    planned_config: String,
}

#[derive(Clone, Default)]
struct Store {
    files: BTreeMap<PathBuf, Vec<u8>>,
    worktrees: Option<Vec<u8>>,
    presets: Option<Vec<u8>>,
}

fn stopped(provider: &dyn PlatformProvider) -> Result<()> {
    if provider.is_claude_desktop_running()? || !provider.running_desktop_args()?.is_empty() {
        return Err(CswError::DesktopRunning);
    }
    Ok(())
}

fn no_pending_launch(provider: &dyn PlatformProvider) -> Result<()> {
    match fs::symlink_metadata(provider.app_data_dir().join(LAUNCH)) {
        Ok(_) => Err(refused(
            "a previous launch was not observed; quit Claude and use csw unshare-code before configuring sharing again",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Serialize CSW launches against sharing, including asynchronous app startup.
/// The durable reservation fails closed if this process exits during startup.
pub fn launch_guard(provider: &dyn PlatformProvider, name: &str) -> Result<Option<File>> {
    if load(provider)?.is_none() && !provider.app_data_dir().join(JOURNAL).try_exists()? {
        return Ok(None);
    }
    let guard = lock(provider)?;
    no_pending_launch(provider)?;
    if is_shared(provider, name)? {
        stopped(provider)?;
    }
    write_private(
        &provider.app_data_dir().join(LAUNCH),
        b"startup not yet observed\n",
    )?;
    Ok(Some(guard))
}

/// Called while the launch guard is held. Never clear a reservation merely
/// because a startup timeout elapsed: the asynchronous launch could still run.
pub fn finish_launch(provider: &dyn PlatformProvider, launched: bool) -> Result<()> {
    if launched {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !provider.is_claude_desktop_running()? && provider.running_desktop_args()?.is_empty()
        {
            if std::time::Instant::now() >= deadline {
                return Err(refused(
                    "Claude startup was not observed; sharing remains blocked until csw unshare-code",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    fs::remove_file(provider.app_data_dir().join(LAUNCH))?;
    sync_directory(&provider.app_data_dir())
}

fn lock(provider: &dyn PlatformProvider) -> Result<File> {
    let root = provider.app_data_dir();
    safe_fs::private_dir(&root)?;
    let path = root.join("native-code.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    file.try_lock()
        .map_err(|_| refused("another synchronization is in progress"))?;
    Ok(file)
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn directories(path: &Path) -> Result<Vec<PathBuf>> {
    safe_fs::reject_symlink(path)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(".csw-stage-") || n.starts_with(".csw-previous-"))
        {
            continue;
        }
        if !entry.file_type()?.is_dir() || !entry.file_name().to_str().is_some_and(uuid) {
            return Err(refused(
                "the account/workspace directory layout is unsupported",
            ));
        }
        result.push(entry.path());
    }
    Ok(result)
}

fn locate(profile: &Profile) -> Result<Location> {
    let desktop = &profile.isolation.desktop_user_data_dir;
    safe_fs::reject_symlink(desktop)?;
    let mut buckets = Vec::new();
    for account in directories(&desktop.join(CATALOG))? {
        for bucket in directories(&account)? {
            buckets.push((account.clone(), bucket));
        }
    }
    if buckets.len() != 1 {
        return Err(refused(
            "open Code once in each environment; exactly one account/workspace bucket is required",
        ));
    }
    let (account, bucket) = buckets
        .pop()
        .ok_or_else(|| refused("no local Code workspace"))?;
    Ok(Location {
        name: profile.profile.name.clone(),
        desktop: desktop.clone(),
        bucket,
        account: account
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| refused("invalid account directory"))?
            .into(),
    })
}

fn locations(manager: &ProfileManager, group: &Group) -> Result<Vec<Location>> {
    if group.version != 1 || group.members.is_empty() || group.members.len() > 64 {
        return Err(refused("unsupported sharing configuration"));
    }
    let source = manager.get_profile(&group.source)?;
    let shared_projects = source
        .isolation
        .cli_config_dir
        .join("projects")
        .canonicalize()?;
    let shared_history = source
        .isolation
        .cli_config_dir
        .join("history.jsonl")
        .canonicalize()?;
    let mut names = vec![group.source.clone()];
    for name in &group.members {
        if names.contains(name) {
            return Err(refused("duplicate environment in sharing configuration"));
        }
        names.push(name.clone());
    }
    let mut result = Vec::new();
    for name in names {
        let profile = manager.get_profile(&name)?;
        if profile
            .isolation
            .cli_config_dir
            .join("projects")
            .canonicalize()?
            != shared_projects
            || profile
                .isolation
                .cli_config_dir
                .join("history.jsonl")
                .canonicalize()?
                != shared_history
        {
            return Err(refused(
                "participants must already share the same local Code transcripts, memory and input history",
            ));
        }
        result.push(locate(&profile)?);
    }
    Ok(result)
}

fn read_regular(path: &Path, remaining: &mut usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > *remaining as u64 {
        return Err(refused(
            "a store contains a link, special file or oversized input",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(refused("hard-linked store files are not supported"));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(refused("store file changed during inspection"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.nlink() != 1 || opened.ino() != metadata.ino() || opened.dev() != metadata.dev() {
            return Err(refused("store file changed during inspection"));
        }
    }
    let mut data = Vec::new();
    file.take(*remaining as u64 + 1).read_to_end(&mut data)?;
    if data.len() > *remaining {
        return Err(refused("store exceeds the size limit"));
    }
    *remaining -= data.len();
    Ok(data)
}

fn collect(root: &Path, relative: &Path, store: &mut Store, remaining: &mut usize) -> Result<()> {
    let dir = root.join(relative);
    safe_fs::reject_symlink(&dir)?;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let text = name
            .to_str()
            .ok_or_else(|| refused("non-UTF-8 store path"))?;
        if ["Cookies", "config.json", ".credentials.json", "Local State"].contains(&text) {
            return Err(refused("authentication storage is not a local Code store"));
        }
        let rel = relative.join(name);
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect(root, &rel, store, remaining)?;
        } else if ty.is_file() {
            if store.files.len() >= MAX_FILES {
                return Err(refused("too many local Code files"));
            }
            let data = read_regular(&entry.path(), remaining)?;
            if entry.path().extension().is_some_and(|s| s == "json") {
                serde_json::from_slice::<serde_json::Value>(&data)
                    .map_err(|_| refused("invalid local Code JSON"))?;
            }
            store.files.insert(rel, data);
        } else {
            return Err(refused(
                "links and special files inside local Code stores are refused",
            ));
        }
    }
    Ok(())
}

impl Store {
    fn read(location: &Location) -> Result<Self> {
        let mut store = Self::default();
        let mut remaining = MAX_BYTES;
        collect(&location.bucket, Path::new(""), &mut store, &mut remaining)?;
        for (name, target) in [
            (WORKTREES, &mut store.worktrees),
            (PRESETS, &mut store.presets),
        ] {
            let path = location.desktop.join(name);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    let bytes = read_regular(&path, &mut remaining)?;
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .map_err(|_| refused("invalid local Code settings JSON"))?;
                    *target = Some(bytes);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(store)
    }
    fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        for (path, data) in &self.files {
            let path = path.to_string_lossy();
            hash.update((path.len() as u64).to_le_bytes());
            hash.update(path.as_bytes());
            hash.update((data.len() as u64).to_le_bytes());
            hash.update(data);
        }
        for data in [&self.worktrees, &self.presets] {
            hash.update([u8::from(data.is_some())]);
            if let Some(data) = data {
                hash.update((data.len() as u64).to_le_bytes());
                hash.update(data);
            }
        }
        format!("{:x}", hash.finalize())
    }
    fn for_account(&self, from: &str, to: &str) -> Result<Self> {
        let mut store = self.clone();
        if from != to
            && let Some(bytes) = &self.presets
        {
            let mut value: serde_json::Value =
                serde_json::from_slice(bytes).map_err(|_| refused("invalid Code presets"))?;
            if let Some(accounts) = value.get_mut("byAccount").and_then(|v| v.as_object_mut()) {
                if let Some(presets) = accounts.get(from).cloned() {
                    accounts.insert(to.into(), presets);
                } else {
                    accounts.remove(to);
                }
            }
            store.presets = Some(serde_json::to_vec(&value)?);
        }
        Ok(store)
    }
    fn snapshot(&self, path: &Path) -> Result<()> {
        safe_fs::private_dir(path)?;
        safe_fs::private_dir(&path.join("bucket"))?;
        for (relative, data) in &self.files {
            write_private(&path.join("bucket").join(relative), data)?;
        }
        for (name, bytes) in [(WORKTREES, &self.worktrees), (PRESETS, &self.presets)] {
            if let Some(bytes) = bytes {
                write_private(&path.join(name), bytes)?;
            }
        }
        Ok(())
    }
    fn apply(&self, location: &Location, tag: &str) -> Result<()> {
        let stage = location.bucket.with_file_name(format!(".csw-stage-{tag}"));
        let old = location
            .bucket
            .with_file_name(format!(".csw-previous-{tag}"));
        if stage.exists() || old.exists() {
            return Err(refused("a previous transaction needs recovery"));
        }
        safe_fs::private_dir(&stage)?;
        for (relative, data) in &self.files {
            write_private(&stage.join(relative), data)?;
        }
        sync_directories(&stage)?;
        safe_fs::reject_symlink(&location.bucket)?;
        let had_bucket = location.bucket.try_exists()?;
        if had_bucket {
            fs::rename(&location.bucket, &old)?;
        }
        if let Err(error) = fs::rename(&stage, &location.bucket) {
            if had_bucket {
                let _ = fs::rename(&old, &location.bucket);
            }
            return Err(error.into());
        }
        // Every original store has already been copied to the durable backup.
        if had_bucket {
            fs::remove_dir_all(old)?;
        }
        sync_directory(
            location
                .bucket
                .parent()
                .ok_or_else(|| refused("invalid bucket path"))?,
        )?;
        for (name, bytes) in [(WORKTREES, &self.worktrees), (PRESETS, &self.presets)] {
            let path = location.desktop.join(name);
            safe_fs::reject_symlink(&path)?;
            if let Some(bytes) = bytes {
                let temp = path.with_file_name(format!(".{name}-{tag}"));
                write_private(&temp, bytes)?;
                fs::rename(temp, path)?;
            } else if path.exists() {
                fs::remove_file(path)?;
            }
        }
        sync_directory(&location.desktop)?;
        Ok(())
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| refused("invalid private path"))?;
    safe_fs::private_dir(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    sync_directory(parent)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn sync_directories(path: &Path) -> Result<()> {
    safe_fs::reject_symlink(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_directories(&entry.path())?;
        }
    }
    sync_directory(path)
}

fn nonce() -> Result<String> {
    Ok(format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| refused("clock unavailable"))?
            .as_nanos(),
        std::process::id()
    ))
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_file_name(format!(".csw-write-{}", nonce()?));
    write_private(&temp, data)?;
    fs::rename(&temp, path)?;
    sync_directory(
        path.parent()
            .ok_or_else(|| refused("invalid private file path"))?,
    )
}

fn journal_locations(manager: &ProfileManager, journal: &Journal) -> Result<Vec<Location>> {
    let names: Vec<_> = std::iter::once(&journal.group.source)
        .chain(&journal.group.members)
        .collect();
    if journal.version != 1
        || journal.group.version != 1
        || journal.group.generation != journal.tag
        || journal.tag.is_empty()
        || journal.tag.len() > 100
        || !journal.tag.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        || names.len() != journal.places.len()
        || names.len() > 65
    {
        return Err(refused("invalid recovery journal"));
    }
    let mut result = Vec::new();
    for (expected, (name, desktop, bucket)) in names.iter().zip(&journal.places) {
        if expected.as_str() != name {
            return Err(refused("recovery participants do not match"));
        }
        let profile = manager.get_profile(name)?;
        if profile.isolation.desktop_user_data_dir != *desktop {
            return Err(refused("recovery environment moved"));
        }
        safe_fs::reject_symlink(desktop)?;
        let catalog = desktop.join(CATALOG);
        let relative = bucket
            .strip_prefix(&catalog)
            .map_err(|_| refused("recovery bucket escapes its environment"))?;
        let parts: Vec<_> = relative.components().collect();
        if parts.len() != 2
            || parts
                .iter()
                .any(|p| !p.as_os_str().to_str().is_some_and(uuid))
        {
            return Err(refused("invalid recovery account/workspace path"));
        }
        let mut checked = desktop.clone();
        for part in [
            CATALOG,
            parts[0]
                .as_os_str()
                .to_str()
                .ok_or_else(|| refused("invalid recovery account"))?,
            parts[1]
                .as_os_str()
                .to_str()
                .ok_or_else(|| refused("invalid recovery workspace"))?,
        ] {
            checked.push(part);
            safe_fs::reject_symlink(&checked)?;
        }
        result.push(Location {
            name: name.clone(),
            desktop: desktop.clone(),
            bucket: bucket.clone(),
            account: parts[0]
                .as_os_str()
                .to_str()
                .ok_or_else(|| refused("invalid recovery account"))?
                .into(),
        });
    }
    Ok(result)
}

fn recover(manager: &ProfileManager, provider: &dyn PlatformProvider) -> Result<()> {
    let root = provider.app_data_dir();
    let path = root.join(JOURNAL);
    if fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(());
    }
    stopped(provider)?;
    no_pending_launch(provider)?;
    let journal: Journal = serde_json::from_slice(&read_regular(&path, &mut 256_000)?)
        .map_err(|_| refused("invalid recovery journal"))?;
    let places = journal_locations(manager, &journal)?;
    let config = root.join(CONFIG);
    let current = match fs::symlink_metadata(&config) {
        Ok(_) => Some(
            String::from_utf8(read_regular(&config, &mut 65536)?)
                .map_err(|_| refused("invalid recovery configuration"))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if current.as_deref() != Some(journal.planned_config.as_str()) {
        if current != journal.prior_config {
            return Err(refused(
                "sharing configuration changed during an interrupted transaction",
            ));
        }
        let backup_root = root.join("native-code-backups");
        safe_fs::reject_symlink(&backup_root)?;
        let backup = backup_root.join(&journal.tag);
        safe_fs::reject_symlink(&backup)?;
        let originals: Vec<_> = places
            .iter()
            .enumerate()
            .map(|(i, p)| {
                safe_fs::reject_symlink(&backup.join(i.to_string()))?;
                Store::read(&Location {
                    name: p.name.clone(),
                    desktop: backup.join(i.to_string()),
                    bucket: backup.join(i.to_string()).join("bucket"),
                    account: p.account.clone(),
                })
            })
            .collect::<Result<_>>()?;
        let authority = places
            .iter()
            .position(|p| p.name == journal.authority)
            .ok_or_else(|| refused("invalid recovery authority"))?;
        let incoming: Vec<_> = places
            .iter()
            .map(|p| originals[authority].for_account(&places[authority].account, &p.account))
            .collect::<Result<_>>()?;
        for (place, store) in places.iter().zip(&incoming) {
            if journal.group.fingerprints.get(&place.name) != Some(&store.fingerprint()) {
                return Err(refused(
                    "recovery snapshot does not match planned transaction",
                ));
            }
        }
        if toml::to_string(&journal.group)? != journal.planned_config {
            return Err(refused(
                "recovery configuration does not match planned transaction",
            ));
        }
        let tag = nonce()?;
        for (store, place) in incoming.iter().zip(&places) {
            store.apply(place, &tag)?;
        }
        atomic_write(&config, journal.planned_config.as_bytes())?;
    }
    // Completed transactions keep their new stores; incomplete ones have now
    // completed the intended copy. Remove only this journal's own staging names.
    for place in &places {
        for prefix in [".csw-stage-", ".csw-previous-"] {
            let leftover = place
                .bucket
                .with_file_name(format!("{prefix}{}", journal.tag));
            safe_fs::reject_symlink(&leftover)?;
            if leftover.exists() {
                fs::remove_dir_all(leftover)?;
            }
        }
        for name in [WORKTREES, PRESETS] {
            let leftover = place.desktop.join(format!(".{name}-{}", journal.tag));
            safe_fs::reject_symlink(&leftover)?;
            if leftover.exists() {
                fs::remove_file(leftover)?;
            }
        }
    }
    fs::remove_file(path)?;
    sync_directory(&root)
}

fn load(provider: &dyn PlatformProvider) -> Result<Option<Group>> {
    let path = provider.app_data_dir().join(CONFIG);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(_) => (),
    }
    let bytes = read_regular(&path, &mut 65536)?;
    let group = toml::from_str(
        std::str::from_utf8(&bytes).map_err(|_| refused("invalid sharing configuration"))?,
    )
    .map_err(|_| refused("invalid sharing configuration"))?;
    Ok(Some(group))
}

fn synchronize(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    group: &mut Group,
    seed: Option<&str>,
) -> Result<PathBuf> {
    stopped(provider)?;
    no_pending_launch(provider)?;
    let places = locations(manager, group)?;
    let originals: Vec<_> = places.iter().map(Store::read).collect::<Result<_>>()?;
    let changed: Vec<_> = places
        .iter()
        .zip(&originals)
        .filter(|(p, s)| group.fingerprints.get(&p.name) != Some(&s.fingerprint()))
        .map(|(p, _)| p.name.as_str())
        .collect();
    let authority = if let Some(seed) = seed {
        seed
    } else {
        match changed.as_slice() {
            [] => group.source.as_str(),
            [one] => one,
            _ => {
                return Err(refused(
                    "multiple environments changed; refusing to choose which complete store to overwrite",
                ));
            }
        }
    };
    let index = places
        .iter()
        .position(|p| p.name == authority)
        .ok_or_else(|| refused("source is not a participant"))?;
    if seed.is_none() && changed.is_empty() {
        return Ok(provider.app_data_dir().join("native-code-backups"));
    }
    let tag = nonce()?;
    let backup_root = provider.app_data_dir().join("native-code-backups");
    safe_fs::private_dir(&backup_root)?;
    let backup = backup_root.join(&tag);
    safe_fs::private_dir(&backup)?;
    for (i, store) in originals.iter().enumerate() {
        store.snapshot(&backup.join(i.to_string()))?;
    }
    write_private(
        &backup.join("locations.json"),
        &serde_json::to_vec(
            &places
                .iter()
                .map(|p| (&p.name, &p.desktop, &p.bucket))
                .collect::<Vec<_>>(),
        )?,
    )?;
    let prior_config = if load(provider)?.is_some() {
        Some(
            String::from_utf8(read_regular(
                &provider.app_data_dir().join(CONFIG),
                &mut 65536,
            )?)
            .map_err(|_| refused("invalid sharing configuration"))?,
        )
    } else {
        None
    };
    if let Some(existing) = &prior_config {
        write_private(&backup.join(CONFIG), existing.as_bytes())?;
    }
    sync_directories(&backup)?;
    sync_directory(&backup_root)?;
    let incoming: Vec<_> = places
        .iter()
        .map(|p| originals[index].for_account(&places[index].account, &p.account))
        .collect::<Result<_>>()?;
    let config = provider.app_data_dir().join(CONFIG);
    let temp = config.with_file_name(format!(".{CONFIG}-{tag}"));
    group.fingerprints = places
        .iter()
        .zip(&incoming)
        .map(|(p, s)| (p.name.clone(), s.fingerprint()))
        .collect();
    group.generation = tag.clone();
    let planned_config = toml::to_string(group)?;
    write_private(&temp, planned_config.as_bytes())?;
    stopped(provider)?;
    let journal = Journal {
        version: 1,
        tag: tag.clone(),
        authority: authority.into(),
        group: group.clone(),
        places: places
            .iter()
            .map(|p| (p.name.clone(), p.desktop.clone(), p.bucket.clone()))
            .collect(),
        prior_config,
        planned_config,
    };
    atomic_write(
        &provider.app_data_dir().join(JOURNAL),
        &serde_json::to_vec(&journal)?,
    )?;
    let result = (|| -> Result<()> {
        for (store, place) in incoming.iter().zip(&places) {
            store.apply(place, &tag)?;
        }
        fs::rename(&temp, &config)?;
        sync_directory(&provider.app_data_dir())?;
        Ok(())
    })();
    if result.is_err() {
        recover(manager, provider)?;
        return result.map(|_| backup);
    }
    recover(manager, provider)?;
    Ok(backup)
}

/// Explicitly seed all participants from one environment. Each destination's
/// existing Code data is backed up before replacement; its login is untouched.
pub fn configure(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    source: &str,
    members: &[String],
) -> Result<PathBuf> {
    let _guard = lock(provider)?;
    recover(manager, provider)?;
    let mut group = Group {
        version: 1,
        generation: String::new(),
        source: source.into(),
        members: members.into(),
        fingerprints: BTreeMap::new(),
    };
    synchronize(manager, provider, &mut group, Some(source))
}

/// Called by the existing switch flow before its active environment changes.
pub fn before_switch(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    target: &str,
) -> Result<()> {
    if load(provider)?.is_none() && !provider.app_data_dir().join(JOURNAL).try_exists()? {
        return Ok(());
    }
    let _guard = lock(provider)?;
    recover(manager, provider)?;
    let Some(mut group) = load(provider)? else {
        return Ok(());
    };
    if group.source == target || group.members.iter().any(|n| n == target) {
        synchronize(manager, provider, &mut group, None)?;
    }
    Ok(())
}

pub fn is_shared(provider: &dyn PlatformProvider, name: &str) -> Result<bool> {
    if fs::symlink_metadata(provider.app_data_dir().join(JOURNAL)).is_ok() {
        return Err(refused(
            "an interrupted transaction must be recovered by switching environments before launching or deleting one",
        ));
    }
    Ok(load(provider)?.is_some_and(|g| g.source == name || g.members.iter().any(|n| n == name)))
}

/// Stop future synchronization without changing any participant's data or login.
pub fn disable(manager: &ProfileManager, provider: &dyn PlatformProvider) -> Result<()> {
    let _guard = lock(provider)?;
    stopped(provider)?;
    match fs::remove_file(provider.app_data_dir().join(LAUNCH)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
        Ok(()) => (),
    }
    recover(manager, provider)?;
    if load(provider)?.is_some() {
        fs::remove_file(provider.app_data_dir().join(CONFIG))?;
    }
    sync_directory(&provider.app_data_dir())
}
