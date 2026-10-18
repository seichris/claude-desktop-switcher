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
const CATALOG: &str = "claude-code-sessions";
const WORKTREES: &str = "git-worktrees.json";
const PRESETS: &str = "ccd-session-presets.json";
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

fn refused(message: &str) -> CswError {
    CswError::Other(format!("Local Code sharing: {message}"))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    version: u32,
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
            if let Some(accounts) = value.get_mut("byAccount").and_then(|v| v.as_object_mut())
                && let Some(presets) = accounts.get(from).cloned()
            {
                accounts.insert(to.into(), presets);
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
        safe_fs::reject_symlink(&location.bucket)?;
        fs::rename(&location.bucket, &old)?;
        if let Err(error) = fs::rename(&stage, &location.bucket) {
            let _ = fs::rename(&old, &location.bucket);
            return Err(error.into());
        }
        // Every original store has already been copied to the durable backup.
        fs::remove_dir_all(old)?;
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
    Ok(())
}

fn load(provider: &dyn PlatformProvider) -> Result<Option<Group>> {
    let path = provider.app_data_dir().join(CONFIG);
    if !path.try_exists()? {
        return Ok(None);
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
    let tag = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| refused("clock unavailable"))?
            .as_nanos(),
        std::process::id()
    );
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
    if let Some(existing) = load(provider)? {
        write_private(&backup.join(CONFIG), toml::to_string(&existing)?.as_bytes())?;
    }
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
    write_private(&temp, toml::to_string(group)?.as_bytes())?;
    stopped(provider)?;
    let result = (|| -> Result<()> {
        for (store, place) in incoming.iter().zip(&places) {
            store.apply(place, &tag)?;
        }
        fs::rename(&temp, &config)?;
        Ok(())
    })();
    if result.is_err() {
        let restore_tag = format!("restore-{tag}");
        for (store, place) in originals.iter().zip(&places) {
            store.apply(place, &restore_tag)?;
        }
        return result.map(|_| backup);
    }
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
    let mut group = Group {
        version: 1,
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
    if load(provider)?.is_none() {
        return Ok(());
    }
    let _guard = lock(provider)?;
    let mut group = load(provider)?.ok_or_else(|| refused("sharing configuration changed"))?;
    if group.source == target || group.members.iter().any(|n| n == target) {
        synchronize(manager, provider, &mut group, None)?;
    }
    Ok(())
}

pub fn is_shared(provider: &dyn PlatformProvider, name: &str) -> Result<bool> {
    Ok(load(provider)?.is_some_and(|g| g.source == name || g.members.iter().any(|n| n == name)))
}
