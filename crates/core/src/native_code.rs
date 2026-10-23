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

mod content;
mod launch;
pub use launch::{
    LaunchGuard, ensure_confirmed_launch, finish_launch, launch_guard, recover_closed_launch,
};

const MAX_CONFIG: usize = 1024 * 1024;
const CONFIG: &str = "native-code.toml";
const JOURNAL: &str = "native-code-journal.json";
const LAUNCH: &str = "native-code-launch-pending";
const CATALOG: &str = "claude-code-sessions";
const WORKTREES: &str = "git-worktrees.json";
const PRESETS: &str = "ccd-session-presets.json";
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 20_000;

fn refused(message: &'static str) -> CswError {
    CswError::LocalCodeSharing(message)
}

fn conflict(message: &'static str) -> CswError {
    CswError::LocalCodeConflict(message)
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Group {
    version: u32,
    #[serde(default)]
    generation: String,
    source: String,
    members: Vec<String>,
    fingerprints: BTreeMap<String, String>,
    #[serde(default)]
    content_fingerprints: BTreeMap<String, String>,
    #[serde(default)]
    known_content: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    identities: BTreeMap<String, Identity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_launch: Option<ConfirmedLaunch>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Identity {
    desktop: PathBuf,
    bucket: PathBuf,
    cli: PathBuf,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ConfirmedLaunch {
    token: String,
    name: String,
    identity: Identity,
}

struct Location {
    name: String,
    desktop: PathBuf,
    bucket: PathBuf,
    account: String,
    cli: PathBuf,
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
    if provider
        .is_claude_desktop_running()
        .map_err(|_| CswError::ProcessInspection)?
        || !provider
            .running_desktop_args()
            .map_err(|_| CswError::ProcessInspection)?
            .is_empty()
    {
        return Err(CswError::DesktopRunning);
    }
    Ok(())
}

fn no_pending_launch(provider: &dyn PlatformProvider) -> Result<()> {
    launch::no_pending_launch(provider)
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
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(refused("invalid operation lock"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(refused("invalid operation lock"));
        }
    }
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
        if result.len() >= 64 {
            return Err(refused("too many account/workspace directories"));
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
        cli: profile.isolation.cli_config_dir.clone(),
        bucket,
        account: account
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| refused("invalid account directory"))?
            .into(),
    })
}

fn locations(manager: &ProfileManager, group: &Group) -> Result<Vec<Location>> {
    if ![1, 2].contains(&group.version) || group.members.is_empty() || group.members.len() > 64 {
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
        let place = locate(&profile)?;
        if group.version == 2 && group.identities.get(&name) != Some(&place.identity()) {
            return Err(refused("a participating environment identity changed"));
        }
        if result
            .iter()
            .any(|p: &Location| p.desktop == place.desktop || p.bucket == place.bucket)
        {
            return Err(refused("participants must have distinct local Code stores"));
        }
        result.push(place);
    }
    Ok(result)
}

impl Location {
    fn validate_paths(&self) -> Result<()> {
        safe_fs::reject_symlink(&self.desktop)?;
        let relative = self
            .bucket
            .strip_prefix(&self.desktop)
            .map_err(|_| refused("a Code bucket escapes its environment"))?;
        let mut checked = self.desktop.clone();
        for component in relative.components() {
            if !matches!(component, std::path::Component::Normal(_)) {
                return Err(refused("invalid Code bucket path"));
            }
            checked.push(component);
            safe_fs::reject_symlink(&checked)?;
        }
        Ok(())
    }
    fn identity(&self) -> Identity {
        Identity {
            desktop: self.desktop.clone(),
            bucket: self.bucket.clone(),
            cli: self.cli.clone(),
        }
    }
}

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 100 && tag.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn backup_store(backup: &Path, name: &str, place: &Location) -> Result<Store> {
    safe_fs::reject_symlink(backup)?;
    let path = backup.join(name);
    safe_fs::reject_symlink(&path)?;
    Store::read(&Location {
        name: place.name.clone(),
        desktop: path.clone(),
        bucket: path.join("bucket"),
        cli: place.cli.clone(),
        account: place.account.clone(),
    })
}

fn bounded_stores(
    stores: impl IntoIterator<Item = Result<Store>>,
    byte_limit: usize,
) -> Result<Vec<Store>> {
    let mut total = 0usize;
    let mut entries = 0usize;
    let mut result = Vec::new();
    for store in stores {
        let store = store?;
        total = total.saturating_add(store.byte_len());
        entries = entries.saturating_add(store.files.len());
        if total > byte_limit || entries > MAX_FILES {
            return Err(refused("sharing group exceeds its size or file limit"));
        }
        result.push(store);
    }
    Ok(result)
}

fn remap_stores(source: &Store, from: &str, places: &[Location]) -> Result<Vec<Store>> {
    remap_stores_with_limit(source, from, places, MAX_BYTES)
}

fn remap_stores_with_limit(
    source: &Store,
    from: &str,
    places: &[Location],
    byte_limit: usize,
) -> Result<Vec<Store>> {
    // Preflight bounds clone allocation even for an invalid legacy snapshot.
    if source.byte_len().saturating_mul(places.len()) > byte_limit
        || source.files.len().saturating_mul(places.len()) > MAX_FILES
    {
        return Err(refused(
            "planned sharing group exceeds its size or file limit",
        ));
    }
    // Account mapping can enlarge presets. Check actual sizes incrementally,
    // before making further copies or writing any transaction data.
    bounded_stores(
        places.iter().map(|p| source.for_account(from, &p.account)),
        byte_limit,
    )
}

/// Reconstruct a legacy generation only from a source snapshot whose remapped
/// complete bytes match every saved fingerprint. Never trust backup timestamps.
fn legacy_baselines(
    provider: &dyn PlatformProvider,
    group: &Group,
    places: &[Location],
) -> Result<Vec<Store>> {
    if !valid_tag(&group.generation) {
        return Err(refused(
            "legacy sharing baseline is unavailable; choose an explicit source",
        ));
    }
    let root = provider.app_data_dir().join("native-code-backups");
    safe_fs::reject_symlink(&root)?;
    let backup = root.join(&group.generation);
    for (i, place) in places.iter().enumerate() {
        let source = backup_store(&backup, &i.to_string(), place)?;
        if source.byte_len().saturating_mul(places.len()) > MAX_BYTES
            || source.files.len().saturating_mul(places.len()) > MAX_FILES
        {
            continue;
        }
        let candidate = remap_stores(&source, &place.account, places)?;
        if places
            .iter()
            .zip(&candidate)
            .all(|(p, s)| group.fingerprints.get(&p.name) == Some(&s.fingerprint()))
        {
            return Ok(candidate);
        }
    }
    Err(refused(
        "legacy sharing baseline cannot be verified; choose an explicit source",
    ))
}

/// Permit startup consuming quit markers only against the exact planned store
/// of this generation. Existing fingerprint semantics and old configurations
/// remain unchanged. Missing or changed backups cannot establish a baseline.
fn consumed_quit_markers_only(
    provider: &dyn PlatformProvider,
    group: &Group,
    places: &[Location],
    index: usize,
    observed: &Store,
) -> Result<bool> {
    if !valid_tag(&group.generation) {
        return Ok(false);
    }
    let root = provider.app_data_dir().join("native-code-backups");
    safe_fs::reject_symlink(&root)?;
    let backup = root.join(&group.generation);
    let selected = backup_store(&backup, "incoming", &places[index])?;
    // The incoming snapshot predates confirmation of the next launch. Its
    // original account must be proved by the saved complete fingerprint,
    // rather than inferred from the current selection or last launch.
    for from in places {
        let baseline = selected.for_account(&from.account, &places[index].account)?;
        if group.fingerprints.get(&places[index].name) == Some(&baseline.fingerprint()) {
            return observed.matches_consumed_quit_markers(&baseline);
        }
    }
    Ok(false)
}

fn authority_index(
    provider: &dyn PlatformProvider,
    group: &Group,
    places: &[Location],
    stores: &[Store],
    seed: Option<&str>,
) -> Result<usize> {
    if let Some(name) = seed {
        return places
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| refused("source is not a participant"));
    }
    if group.version == 1 {
        let changed: Vec<_> = places
            .iter()
            .zip(stores)
            .enumerate()
            .filter(|(_, (p, s))| group.fingerprints.get(&p.name) != Some(&s.fingerprint()))
            .map(|(i, _)| i)
            .collect();
        if changed.len() <= 1 {
            return changed
                .first()
                .copied()
                .or_else(|| places.iter().position(|p| p.name == group.source))
                .ok_or_else(|| refused("missing source"));
        }
    }
    let contents: Vec<_> = stores
        .iter()
        .map(Store::content_fingerprint)
        .collect::<Result<_>>()?;
    if let Some(launch) = &group.last_launch {
        let index = places
            .iter()
            .position(|p| p.name == launch.name && p.identity() == launch.identity)
            .ok_or_else(|| refused("last confirmed launch identity no longer matches"))?;
        for (i, place) in places.iter().enumerate() {
            if i != index
                && group.content_fingerprints.get(&place.name) != Some(&contents[i])
                && !group
                    .known_content
                    .get(&place.name)
                    .is_some_and(|known| known.contains(&contents[i]))
                && contents[i]
                    != stores[index]
                        .for_account(&places[index].account, &place.account)?
                        .content_fingerprint()?
                && !consumed_quit_markers_only(provider, group, places, i, &stores[i])?
            {
                return Err(conflict(
                    "an environment outside the last confirmed launch has unknown content changes; quit all writers and explicitly choose a source",
                ));
            }
        }
        return Ok(index);
    }
    let baselines = if group.version == 1 {
        Some(legacy_baselines(provider, group, places)?)
    } else {
        None
    };
    let mut changed = Vec::new();
    for (i, place) in places.iter().enumerate() {
        let baseline = if let Some(stores) = &baselines {
            Some(stores[i].content_fingerprint()?)
        } else {
            group.content_fingerprints.get(&place.name).cloned()
        };
        if baseline.as_ref() != Some(&contents[i]) {
            changed.push(i);
        }
    }
    match changed.as_slice() {
        [] => places
            .iter()
            .position(|p| p.name == group.source)
            .ok_or_else(|| refused("missing source")),
        [one] => Ok(*one),
        _ => Err(conflict(
            "multiple environments changed without confirmed launch provenance; explicitly choose a source",
        )),
    }
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

fn collect(
    root: &Path,
    relative: &Path,
    store: &mut Store,
    remaining: &mut usize,
    entries: &mut usize,
) -> Result<()> {
    if relative.components().count() > 64 {
        return Err(refused("local Code directory tree is too deep"));
    }
    let dir = root.join(relative);
    safe_fs::reject_symlink(&dir)?;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        *entries = entries
            .checked_sub(1)
            .ok_or_else(|| refused("too many local Code entries"))?;
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
            collect(root, &rel, store, remaining, entries)?;
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
    fn byte_len(&self) -> usize {
        self.files.values().map(Vec::len).sum::<usize>()
            + self.worktrees.as_ref().map_or(0, Vec::len)
            + self.presets.as_ref().map_or(0, Vec::len)
    }
    fn read(location: &Location) -> Result<Self> {
        location.validate_paths()?;
        let mut store = Self::default();
        let mut remaining = MAX_BYTES;
        collect(
            &location.bucket,
            Path::new(""),
            &mut store,
            &mut remaining,
            &mut { MAX_FILES },
        )?;
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
    fn apply(
        &self,
        location: &Location,
        tag: &str,
        provider: &dyn PlatformProvider,
        expected: &Store,
    ) -> Result<()> {
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
        stopped(provider)?;
        location.validate_paths()?;
        if Store::read(location)?.fingerprint() != expected.fingerprint() {
            return Err(conflict(
                "a local Code store changed while replacement was staged",
            ));
        }
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
        let mut expected = expected.clone();
        expected.files = self.files.clone();
        for (name, bytes) in [(WORKTREES, &self.worktrees), (PRESETS, &self.presets)] {
            let path = location.desktop.join(name);
            safe_fs::reject_symlink(&path)?;
            if let Some(bytes) = bytes {
                let temp = path.with_file_name(format!(".{name}-{tag}"));
                write_private(&temp, bytes)?;
                stopped(provider)?;
                location.validate_paths()?;
                if Store::read(location)?.fingerprint() != expected.fingerprint() {
                    return Err(conflict(
                        "a local Code store changed while settings were staged",
                    ));
                }
                fs::rename(temp, path)?;
            } else {
                stopped(provider)?;
                location.validate_paths()?;
                if Store::read(location)?.fingerprint() != expected.fingerprint() {
                    return Err(conflict(
                        "a local Code store changed before settings removal",
                    ));
                }
                if path.try_exists()? {
                    fs::remove_file(path)?;
                }
            }
            if name == WORKTREES {
                expected.worktrees = self.worktrees.clone();
            } else {
                expected.presets = self.presets.clone();
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
    if ![1, 2].contains(&journal.version)
        || ![1, 2].contains(&journal.group.version)
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
    let mut seen = std::collections::BTreeSet::new();
    for (expected, (name, desktop, bucket)) in names.iter().zip(&journal.places) {
        if expected.as_str() != name || !seen.insert(name) {
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
        if journal.group.version == 2
            && journal.group.identities.get(name)
                != Some(&Identity {
                    desktop: desktop.clone(),
                    bucket: bucket.clone(),
                    cli: profile.isolation.cli_config_dir.clone(),
                })
        {
            return Err(refused("recovery environment identity changed"));
        }
        result.push(Location {
            name: name.clone(),
            desktop: desktop.clone(),
            bucket: bucket.clone(),
            cli: profile.isolation.cli_config_dir.clone(),
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
    let journal: Journal = serde_json::from_slice(&read_regular(&path, &mut (3 * MAX_CONFIG))?)
        .map_err(|_| refused("invalid recovery journal"))?;
    let places = journal_locations(manager, &journal)?;
    let config = root.join(CONFIG);
    let current = match fs::symlink_metadata(&config) {
        Ok(_) => Some(
            String::from_utf8(read_regular(&config, &mut { MAX_CONFIG })?)
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
        let originals = bounded_stores(
            places.iter().enumerate().map(|(i, p)| {
                safe_fs::reject_symlink(&backup.join(i.to_string()))?;
                Store::read(&Location {
                    name: p.name.clone(),
                    desktop: backup.join(i.to_string()),
                    bucket: backup.join(i.to_string()).join("bucket"),
                    account: p.account.clone(),
                    cli: p.cli.clone(),
                })
            }),
            MAX_BYTES,
        )?;
        let authority = places
            .iter()
            .position(|p| p.name == journal.authority)
            .ok_or_else(|| refused("invalid recovery authority"))?;
        let selected = if journal.version == 2 {
            backup_store(&backup, "incoming", &places[authority])?
        } else {
            originals[authority].clone()
        };
        let incoming = remap_stores(&selected, &places[authority].account, &places)?;
        for (place, store) in places.iter().zip(&incoming) {
            if journal.group.fingerprints.get(&place.name) != Some(&store.fingerprint()) {
                return Err(refused(
                    "recovery snapshot does not match planned transaction",
                ));
            }
        }
        if toml::from_str::<Group>(&journal.planned_config)
            .map_err(|_| refused("invalid planned recovery configuration"))?
            != journal.group
        {
            return Err(refused(
                "recovery configuration does not match planned transaction",
            ));
        }
        // Bucket replacement is atomic; individual settings can be on either
        // side of a crash. Unknown bytes must not be silently overwritten.
        for ((store, place), original) in incoming.iter().zip(&places).zip(&originals) {
            stopped(provider)?;
            let mut read_place = Location {
                name: place.name.clone(),
                desktop: place.desktop.clone(),
                bucket: place.bucket.clone(),
                cli: place.cli.clone(),
                account: place.account.clone(),
            };
            if !place.bucket.try_exists()? {
                read_place.bucket = place
                    .bucket
                    .with_file_name(format!(".csw-previous-{}", journal.tag));
            }
            if !Store::read(&read_place)?.transaction_state(original, store) {
                return Err(CswError::LocalCodeRecoveryConflict(
                    "unknown local Code updates appeared during interrupted synchronization; recovery is paused",
                ));
            }
        }
        for ((store, place), original) in incoming.iter().zip(&places).zip(&originals) {
            stopped(provider)?;
            // Revalidate each target again immediately before its replacement.
            let prior = place
                .bucket
                .with_file_name(format!(".csw-previous-{}", journal.tag));
            if !place.bucket.try_exists()? {
                safe_fs::reject_symlink(&prior)?;
                fs::rename(&prior, &place.bucket)?;
            }
            let expected = Store::read(place)?;
            if !expected.transaction_state(original, store) {
                return Err(CswError::LocalCodeRecoveryConflict(
                    "a local Code store changed during recovery",
                ));
            }
            for prefix in [".csw-stage-", ".csw-previous-"] {
                let debris = place
                    .bucket
                    .with_file_name(format!("{prefix}{}", journal.tag));
                safe_fs::reject_symlink(&debris)?;
                if debris.try_exists()? {
                    fs::remove_dir_all(debris)?;
                }
            }
            for name in [WORKTREES, PRESETS] {
                let debris = place.desktop.join(format!(".{name}-{}", journal.tag));
                safe_fs::reject_symlink(&debris)?;
                if debris.try_exists()? {
                    fs::remove_file(debris)?;
                }
            }
            store.apply(place, &journal.tag, provider, &expected)?;
        }
        stopped(provider)?;
        for (store, place) in incoming.iter().zip(&places) {
            if Store::read(place)?.fingerprint() != store.fingerprint() {
                return Err(CswError::LocalCodeRecoveryConflict(
                    "a local Code store changed before recovery committed",
                ));
            }
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
    let bytes = read_regular(&path, &mut { MAX_CONFIG })?;
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
    let originals = bounded_stores(places.iter().map(Store::read), MAX_BYTES)?;
    let index = authority_index(provider, group, &places, &originals, seed)?;
    let authority = places[index].name.clone();
    let mut selected = originals[index].clone();
    if seed.is_none() {
        selected.merge_housekeeping(&originals)?;
    }
    let incoming = remap_stores(&selected, &places[index].account, &places)?;
    if seed.is_none()
        && group.version == 2
        && originals
            .iter()
            .zip(&incoming)
            .all(|(a, b)| a.fingerprint() == b.fingerprint())
    {
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
            String::from_utf8(read_regular(&provider.app_data_dir().join(CONFIG), &mut {
                MAX_CONFIG
            })?)
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
    // The selected store includes merged housekeeping, so recovery needs its
    // exact bytes as well as each original (which always remains untouched).
    selected.snapshot(&backup.join("incoming"))?;
    sync_directories(&backup)?;
    sync_directory(&backup_root)?;
    let config = provider.app_data_dir().join(CONFIG);
    let temp = config.with_file_name(format!(".{CONFIG}-{tag}"));
    for (place, store) in places.iter().zip(&incoming) {
        let known = group.known_content.entry(place.name.clone()).or_default();
        if let Some(prior) = group.content_fingerprints.get(&place.name) {
            known.retain(|k| k != prior);
            known.push(prior.clone());
        }
        let next = store.content_fingerprint()?;
        known.retain(|k| k != &next);
        known.push(next.clone());
        if known.len() > 16 {
            known.drain(..known.len() - 16);
        }
        group.content_fingerprints.insert(place.name.clone(), next);
        group
            .identities
            .insert(place.name.clone(), place.identity());
    }
    group.version = 2;
    group.fingerprints = places
        .iter()
        .zip(&incoming)
        .map(|(p, s)| (p.name.clone(), s.fingerprint()))
        .collect();
    group.generation = tag.clone();
    let planned_config = toml::to_string(group)?;
    write_private(&temp, planned_config.as_bytes())?;
    stopped(provider)?;
    // Revalidate identities and the exact source/destination bytes after backup.
    let checked = locations(manager, group)?;
    for (place, original) in checked.iter().zip(&originals) {
        if Store::read(place)?.fingerprint() != original.fingerprint() {
            return Err(conflict(
                "a local Code store changed during backup; no replacement was started",
            ));
        }
    }
    let observed_config = match fs::symlink_metadata(&config) {
        Ok(_) => Some(
            String::from_utf8(read_regular(&config, &mut { MAX_CONFIG })?)
                .map_err(|_| refused("invalid sharing configuration"))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if observed_config != prior_config {
        return Err(conflict("sharing configuration changed during backup"));
    }
    let journal = Journal {
        version: 2,
        tag: tag.clone(),
        authority,
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
        for ((store, place), original) in incoming.iter().zip(&places).zip(&originals) {
            stopped(provider)?;
            if Store::read(place)?.fingerprint() != original.fingerprint() {
                return Err(conflict("a local Code store changed before replacement"));
            }
            store.apply(place, &tag, provider, original)?;
        }
        stopped(provider)?;
        for (store, place) in incoming.iter().zip(&places) {
            if Store::read(place)?.fingerprint() != store.fingerprint() {
                return Err(conflict("a local Code store changed during replacement"));
            }
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
        content_fingerprints: BTreeMap::new(),
        known_content: BTreeMap::new(),
        identities: BTreeMap::new(),
        last_launch: None,
    };
    synchronize(manager, provider, &mut group, Some(source))
}

/// Enrollment from the GUI is additive. Preserve every existing participant and
/// carry forward its latest changes before seeding the newly confirmed member.
/// Unlike `configure`, this can never silently drop an existing sharing member.
pub fn add_member(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    name: &str,
    replace: bool,
) -> Result<PathBuf> {
    if !replace {
        return Err(refused(
            "confirm backing up and replacing this environment's local Code stores",
        ));
    }
    let _guard = lock(provider)?;
    stopped(provider)?;
    no_pending_launch(provider)?;
    recover(manager, provider)?;
    let profile = manager.get_profile(name)?;
    let source = &profile.sharing.source.profile;
    if name == source {
        return Err(refused("choose a separate environment to add"));
    }
    let existing = load(provider)?;
    let mut group = existing.clone().unwrap_or(Group {
        version: 1,
        generation: String::new(),
        source: source.clone(),
        members: Vec::new(),
        fingerprints: BTreeMap::new(),
        content_fingerprints: BTreeMap::new(),
        known_content: BTreeMap::new(),
        identities: BTreeMap::new(),
        last_launch: None,
    });
    if group.source == name || group.members.iter().any(|n| n == name) {
        return Err(refused(
            "this environment already participates in local Code sharing",
        ));
    }
    if group.source != *source && !group.members.contains(source) {
        return Err(refused(
            "the sharing source is outside the current local Code group",
        ));
    }
    // Validate the new member separately; it has no synchronized identity yet.
    Store::read(&locate(&profile)?)?;
    // Validate all stores/links before changing the existing group at all.
    let mut prospective = group.clone();
    prospective.members.push(name.into());
    prospective
        .identities
        .insert(name.into(), locate(&profile)?.identity());
    for place in locations(manager, &prospective)? {
        Store::read(&place)?;
    }
    if let Some(mut current) = existing {
        synchronize(manager, provider, &mut current, None)?;
        group = current;
    }
    group.members.push(name.into());
    group
        .identities
        .insert(name.into(), locate(&profile)?.identity());
    let authority = group
        .last_launch
        .as_ref()
        .map_or(&group.source, |l| &l.name)
        .clone();
    synchronize(manager, provider, &mut group, Some(&authority))
}

#[derive(Serialize)]
pub struct SharingStatus {
    pub enabled: bool,
    pub eligible: bool,
    pub source: String,
    pub participants: usize,
}

/// Read declarations only. Do not infer sign-in state from authentication files.
pub fn sharing_status(
    manager: &ProfileManager,
    provider: &dyn PlatformProvider,
    name: &str,
) -> Result<SharingStatus> {
    let profile = manager.get_profile(name)?;
    let group = load(provider)?;
    let enabled = group
        .as_ref()
        .is_some_and(|g| g.source == name || g.members.iter().any(|n| n == name));
    Ok(SharingStatus {
        enabled,
        eligible: name != profile.sharing.source.profile
            && profile.sharing.cli_project_memory == crate::profile::SharingMode::Share
            && profile.sharing.cli_history == crate::profile::SharingMode::Share,
        source: profile.sharing.source.profile,
        participants: if enabled {
            group.map_or(0, |g| g.members.len() + 1)
        } else {
            0
        },
    })
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

#[cfg(test)]
mod commit_tests {
    use super::*;
    use crate::platform::mock::MockPlatformProvider;

    fn place(root: &Path, name: &str, account: &str) -> Location {
        Location {
            name: name.into(),
            desktop: root.into(),
            bucket: root.join("bucket"),
            account: account.into(),
            cli: root.join("cli"),
        }
    }

    #[test]
    fn staged_replacement_rechecks_bytes_and_process_state_before_commit() {
        for running in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let location = place(&tmp.path().join("desktop"), "synthetic", "account");
            fs::create_dir_all(&location.bucket).unwrap();
            fs::write(
                location.bucket.join("thread.json"),
                b"{\"title\":\"before\"}",
            )
            .unwrap();
            let prior = Store::read(&location).unwrap();
            let mut next = prior.clone();
            next.files
                .insert("thread.json".into(), b"{\"title\":\"incoming\"}".to_vec());
            // A new write between the caller's snapshot and the apply commit.
            fs::write(
                location.bucket.join("thread.json"),
                b"{\"title\":\"unbacked update\"}",
            )
            .unwrap();
            let provider = MockPlatformProvider::new(
                location.desktop.clone(),
                location.cli.clone(),
                tmp.path().join("app"),
            )
            .with_desktop_running(running);
            let result = next.apply(&location, "test", &provider, &prior);
            assert!(result.is_err());
            // The stage is durable, proving the check runs at the commit point.
            assert!(
                location
                    .bucket
                    .with_file_name(".csw-stage-test")
                    .join("thread.json")
                    .exists()
            );
            assert_eq!(
                fs::read(location.bucket.join("thread.json")).unwrap(),
                b"{\"title\":\"unbacked update\"}"
            );
            assert!(
                !location
                    .bucket
                    .with_file_name(".csw-previous-test")
                    .exists()
            );
        }
    }

    #[test]
    fn preset_expansion_is_bounded_after_account_remapping() {
        let tmp = tempfile::tempdir().unwrap();
        let places = [
            place(&tmp.path().join("one"), "one", "a"),
            place(&tmp.path().join("two"), "two", "b"),
        ];
        let source = Store {
            presets: Some(
                serde_json::to_vec(
                    &serde_json::json!({"byAccount":{"a":{"body":"x".repeat(100)}}}),
                )
                .unwrap(),
            ),
            ..Store::default()
        };
        let pre_remap_total = source.byte_len() * places.len();
        assert!(remap_stores_with_limit(&source, "a", &places, pre_remap_total).is_err());
        assert!(remap_stores_with_limit(&source, "a", &places, pre_remap_total * 2).is_ok());
        assert!(!places[0].bucket.exists());
    }
}
