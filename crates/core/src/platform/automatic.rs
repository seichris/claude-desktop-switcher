//! Safe automatic quitting is limited to a reviewed vendor build and a launch
//! whose quit guard CSW explicitly enabled. No signals or private app storage.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
use core_foundation::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::boolean::{CFBoolean, CFBooleanGetTypeID, kCFBooleanTrue};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringGetLength, CFStringGetTypeID};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CswError, Result};
use crate::platform::PlatformProvider;

const APP: &str = "/Applications/Claude.app";
const EXECUTABLE: &str = "/Applications/Claude.app/Contents/MacOS/Claude";
const REVIEWED_VERSION: &str = "2.42304.0";
const REVIEWED_ASAR: &str = "bfa22ac393906762fa04b9607ff202907405b089c1ae7db257159202f31bb3d0";
const REQUIREMENT: &str = "anchor apple generic and identifier \"com.anthropic.claudefordesktop\" and certificate leaf[subject.OU] = \"Q6L2SF6YDW\"";
const PROOF_DIRECTORY: &str = "automatic-switch";
const PROOF_FILE: &str = "safe-launch.json";
const INITIAL_GUARD_TIMEOUT: Duration = Duration::from_secs(10);

/// This vendor build installs its task-aware guard asynchronously, after main
/// web-content load or a 60-second timeout. Neither app launch completion, wall
/// time nor AXLoaded attests that installation. Until a positive, process-bound
/// witness is available, automatic quit is quarantined. Keep this check at the
/// action boundary too: no future caller may turn identity proof into readiness.
fn require_guard_readiness() -> Result<()> {
    Err(refuse("automatic_guard_unverified"))
}

fn refuse(message: &str) -> CswError {
    CswError::Other(message.into())
}

/// Accessibility is checked without prompting or reading another app's UI.
pub fn trusted() -> bool {
    // SAFETY: Public ApplicationServices API has no arguments or ownership.
    unsafe { AXIsProcessTrusted() }
}

/// Only call in response to the user's explicit Accessibility permission action.
pub fn request_permission() -> Result<()> {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    // SAFETY: kCFBooleanTrue is an immortal Core Foundation Boolean.
    let value = unsafe { core_foundation::boolean::CFBoolean::wrap_under_get_rule(kCFBooleanTrue) };
    let options = CFDictionary::from_CFType_pairs(&[(key, value)]);
    // SAFETY: options remains retained for this synchronous public API call.
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) };
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitProgress {
    /// Normal quit requested; the native quit guard or app exit is still pending.
    Running,
    /// Claude's own guard now waits for all its active work to finish.
    Waiting,
    /// The exact original app exited; no replacement Claude process exists.
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchProof {
    version: u32,
    pid: u32,
    launch_date_bits: u64,
    desktop: PathBuf,
    vendor_version: String,
    asar_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: SystemTime,
}

/// Owns the original app object and immutable launch proof. Holding this value
/// does not request a quit. Each operation revalidates the same app instance.
pub struct SafeQuit<'a> {
    provider: &'a dyn PlatformProvider,
    app: Retained<NSRunningApplication>,
    proof: LaunchProof,
    vendor_stamp: Vec<FileStamp>,
    requested: Option<Instant>,
    waiting: bool,
}

impl<'a> SafeQuit<'a> {
    pub fn prepare(
        provider: &'a dyn PlatformProvider,
        pid: u32,
        expected_desktop: &Path,
    ) -> Result<Self> {
        require_guard_readiness()?;
        if !trusted() {
            return Err(refuse("automatic_accessibility_required"));
        }
        validate_desktop(expected_desktop)?;
        let proof = read_proof(provider)?;
        if proof.pid != pid || proof.desktop != expected_desktop {
            return Err(unmanaged());
        }
        let app = current_app(pid)?;
        let vendor_stamp = verify_vendor(provider)?;
        let quit = Self {
            provider,
            app,
            proof,
            vendor_stamp,
            requested: None,
            waiting: false,
        };
        quit.validate_running()?;
        let launched = quit.app.launchDate().ok_or_else(unmanaged)?;
        if !quit.app.isFinishedLaunching() || launched.timeIntervalSinceNow() > -5.0 {
            return Err(refuse("automatic_startup"));
        }
        if !native_modals(pid)?.is_empty() {
            return Err(refuse("automatic_unknown_dialog"));
        }
        Ok(quit)
    }

    /// Send exactly one normal application quit request. Never signal a process,
    /// never use forceTerminate, and never invoke the guard-bypassing menu path.
    pub fn request(&mut self) -> Result<()> {
        require_guard_readiness()?;
        if self.requested.is_some() {
            return Err(refuse("automatic_quit_refused"));
        }
        self.validate_running()?;
        self.vendor_stamp = verify_vendor(self.provider)?;
        if !native_modals(self.proof.pid)?.is_empty() {
            return Err(refuse("automatic_unknown_dialog"));
        }
        if !self.app.terminate() {
            return Err(refuse("automatic_quit_refused"));
        }
        self.requested = Some(Instant::now());
        Ok(())
    }

    /// Call no faster than once every 500 ms. After selecting the native wait
    /// option, only observe until Claude exits; never repeat the quit request.
    pub fn poll(&mut self) -> Result<QuitProgress> {
        let requested = self
            .requested
            .ok_or_else(|| refuse("automatic_quit_refused"))?;
        if self.app.isTerminated() {
            if !self.provider.running_desktop_processes()?.is_empty() {
                return Err(refuse("automatic_identity_changed"));
            }
            return Ok(QuitProgress::Exited);
        }
        self.validate_running()?;
        if self.waiting {
            // Native scheduling includes responding, held-input and background
            // work. No text, window or chat inspection is needed while waiting.
            return Ok(QuitProgress::Waiting);
        }
        let mut modals = native_modals(self.proof.pid)?;
        if modals.len() > 1 {
            return Err(refuse("automatic_unknown_dialog"));
        }
        if let Some(modal) = modals.pop() {
            let mut contents = ModalContents::default();
            collect_modal(&modal, self.proof.pid, 0, &mut contents)?;
            let labels: Vec<String> = contents
                .buttons
                .iter()
                .map(|(label, _)| label.clone())
                .collect();
            let wait_label = recognized_guard(&contents.texts, &labels)
                .ok_or_else(|| refuse("automatic_unknown_dialog"))?;
            let (_, button) = contents
                .buttons
                .into_iter()
                .find(|(label, _)| label == wait_label)
                .ok_or_else(|| refuse("automatic_unknown_dialog"))?;
            self.validate_running()?;
            self.vendor_stamp = verify_vendor(self.provider)?;
            if button.string("AXRole")?.as_deref() != Some("AXButton")
                || button.string("AXTitle")?.as_deref() != Some(wait_label)
                || button.boolean("AXEnabled")? != Some(true)
            {
                return Err(refuse("automatic_unknown_dialog"));
            }
            button.check_pid(self.proof.pid)?;
            button.press()?;
            self.waiting = true;
            return Ok(QuitProgress::Waiting);
        }
        if requested.elapsed() >= INITIAL_GUARD_TIMEOUT {
            return Err(refuse("automatic_quit_timeout"));
        }
        Ok(QuitProgress::Running)
    }

    fn validate_running(&self) -> Result<()> {
        if !trusted() {
            return Err(refuse("automatic_accessibility_required"));
        }
        if self.app.isTerminated() || read_proof(self.provider)? != self.proof {
            return Err(unmanaged());
        }
        validate_proof(&self.proof, &self.app)?;
        // Re-resolve the current Launch Services instance as well as retaining
        // the original object. PID reuse must not confer the old launch proof.
        let current = current_app(self.proof.pid)?;
        validate_proof(&self.proof, &current)?;
        if current != self.app {
            return Err(refuse("automatic_identity_changed"));
        }
        let processes = self.provider.running_desktop_processes()?;
        if processes.len() != 1
            || processes[0].0 != self.proof.pid
            || !exact_launch_args(&processes[0].1, &self.proof.desktop)
        {
            return Err(unmanaged());
        }
        if vendor_stamp()? != self.vendor_stamp {
            return Err(refuse("automatic_identity_changed"));
        }
        Ok(())
    }
}

fn unmanaged() -> CswError {
    refuse("automatic_unmanaged_launch")
}

fn validate_desktop(desktop: &Path) -> Result<()> {
    let text = desktop.to_str().ok_or_else(unmanaged)?;
    if !desktop.is_absolute()
        || text.chars().any(char::is_control)
        || desktop
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(unmanaged());
    }
    Ok(())
}

fn exact_launch_args(args: &str, desktop: &Path) -> bool {
    args == format!("{EXECUTABLE} --user-data-dir={}", desktop.display())
}

fn current_app(pid: u32) -> Result<Retained<NSRunningApplication>> {
    let pid = i32::try_from(pid).map_err(|_| unmanaged())?;
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid).ok_or_else(unmanaged)
}

fn launch_bits(app: &NSRunningApplication) -> Result<u64> {
    let date = app.launchDate().ok_or_else(unmanaged)?;
    let interval = date.timeIntervalSinceReferenceDate();
    if !interval.is_finite() || interval <= 0.0 {
        return Err(unmanaged());
    }
    Ok(interval.to_bits())
}

fn validate_proof(proof: &LaunchProof, app: &NSRunningApplication) -> Result<()> {
    validate_proof_policy(proof)?;
    if app.processIdentifier() != i32::try_from(proof.pid).map_err(|_| unmanaged())?
        || app.isTerminated()
        || launch_bits(app)? != proof.launch_date_bits
        || app
            .bundleIdentifier()
            .as_deref()
            .map(ToString::to_string)
            .as_deref()
            != Some("com.anthropic.claudefordesktop")
        || app
            .bundleURL()
            .and_then(|url| url.path())
            .map(|path| path.to_string())
            .as_deref()
            != Some(APP)
        || app
            .executableURL()
            .and_then(|url| url.path())
            .map(|path| path.to_string())
            .as_deref()
            != Some(EXECUTABLE)
    {
        return Err(unmanaged());
    }
    validate_desktop(&proof.desktop)
}

fn validate_proof_policy(proof: &LaunchProof) -> Result<()> {
    let launch_date = f64::from_bits(proof.launch_date_bits);
    if proof.version != 1
        || proof.pid == 0
        || i32::try_from(proof.pid).is_err()
        || proof.vendor_version != REVIEWED_VERSION
        || proof.asar_sha256 != REVIEWED_ASAR
        || !launch_date.is_finite()
        || launch_date <= 0.0
    {
        return Err(unmanaged());
    }
    validate_desktop(&proof.desktop)
}

/// Called only after the controlled `open --env ...=0` launch below. A process
/// that existed before that launch can never be granted a new launch proof.
pub fn record_safe_launch(
    provider: &dyn PlatformProvider,
    desktop: &Path,
    previous_pids: &[u32],
) -> Result<()> {
    validate_desktop(desktop)?;
    verify_vendor(provider)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let processes = provider.running_desktop_processes()?;
        if processes.len() == 1 {
            let (pid, args) = &processes[0];
            if !previous_pids.contains(pid) && exact_launch_args(args, desktop) {
                let app = current_app(*pid)?;
                let proof = LaunchProof {
                    version: 1,
                    pid: *pid,
                    launch_date_bits: launch_bits(&app)?,
                    desktop: desktop.into(),
                    vendor_version: REVIEWED_VERSION.into(),
                    asar_sha256: REVIEWED_ASAR.into(),
                };
                validate_proof(&proof, &app)?;
                return write_proof(provider, &proof);
            }
        }
        if Instant::now() >= deadline {
            return Err(unmanaged());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn private_directory(provider: &dyn PlatformProvider, create: bool) -> Result<PathBuf> {
    let root = provider.app_data_dir();
    let root_meta = fs::symlink_metadata(&root)?;
    if !root_meta.is_dir()
        || root_meta.file_type().is_symlink()
        || root_meta.uid() != unsafe { libc::geteuid() }
        || root_meta.mode() & 0o022 != 0
    {
        return Err(refuse("automatic_untrusted_storage"));
    }
    let path = root.join(PROOF_DIRECTORY);
    if create && !path.try_exists()? {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&path)?;
    }
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.mode() & 0o077 != 0
        || meta.uid() != unsafe { libc::geteuid() }
    {
        return Err(refuse("automatic_untrusted_storage"));
    }
    Ok(path)
}

fn read_proof(provider: &dyn PlatformProvider) -> Result<LaunchProof> {
    let dir = private_directory(provider, false).map_err(|_| unmanaged())?;
    let path = dir.join(PROOF_FILE);
    let before = fs::symlink_metadata(&path).map_err(|_| unmanaged())?;
    if !before.is_file()
        || before.file_type().is_symlink()
        || before.nlink() != 1
        || before.mode() & 0o077 != 0
        || before.uid() != unsafe { libc::geteuid() }
        || before.len() > 16_384
    {
        return Err(unmanaged());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| unmanaged())?;
    let opened = file.metadata()?;
    if before.ino() != opened.ino() || before.dev() != opened.dev() || opened.nlink() != 1 {
        return Err(unmanaged());
    }
    let mut bytes = Vec::new();
    file.take(16_385).read_to_end(&mut bytes)?;
    if bytes.len() > 16_384 {
        return Err(unmanaged());
    }
    serde_json::from_slice(&bytes).map_err(|_| unmanaged())
}

fn write_proof(provider: &dyn PlatformProvider, proof: &LaunchProof) -> Result<()> {
    let dir = private_directory(provider, true)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| refuse("automatic_untrusted_storage"))?
        .as_nanos();
    let temp = dir.join(format!(".proof-{}-{nonce}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec(proof)?)?;
    file.sync_all()?;
    fs::rename(&temp, dir.join(PROOF_FILE))?;
    File::open(dir)?.sync_all()?;
    Ok(())
}

fn file_stamp(path: &Path) -> Result<FileStamp> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.nlink() != 1 {
        return Err(refuse("automatic_identity_changed"));
    }
    Ok(FileStamp {
        device: meta.dev(),
        inode: meta.ino(),
        size: meta.len(),
        modified: meta.modified()?,
    })
}

fn vendor_stamp() -> Result<Vec<FileStamp>> {
    [
        EXECUTABLE,
        "/Applications/Claude.app/Contents/Resources/app.asar",
        "/Applications/Claude.app/Contents/Info.plist",
    ]
    .iter()
    .map(|path| file_stamp(Path::new(path)))
    .collect()
}

fn verify_vendor(provider: &dyn PlatformProvider) -> Result<Vec<FileStamp>> {
    if provider.claude_desktop_app_path() != Path::new(APP)
        || fs::symlink_metadata(APP)?.file_type().is_symlink()
    {
        return Err(refuse("automatic_unsupported_build"));
    }
    let before = vendor_stamp()?;
    let version = Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            "Print CFBundleShortVersionString",
            "/Applications/Claude.app/Contents/Info.plist",
        ])
        .stdin(Stdio::null())
        .output()?;
    if !version.status.success()
        || String::from_utf8_lossy(&version.stdout).trim() != REVIEWED_VERSION
    {
        return Err(refuse("automatic_unsupported_build"));
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open("/Applications/Claude.app/Contents/Resources/app.asar")?;
    if file.metadata()?.len() > 80 * 1024 * 1024 {
        return Err(refuse("automatic_unsupported_build"));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65_536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    if format!("{:x}", hasher.finalize()) != REVIEWED_ASAR {
        return Err(refuse("automatic_unsupported_build"));
    }
    let signature = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "-R", REQUIREMENT, APP])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !signature.success() || vendor_stamp()? != before {
        return Err(refuse("automatic_unsupported_build"));
    }
    Ok(before)
}

fn session_count(text: &str, prefix: &str, suffix: &str) -> bool {
    text.strip_prefix(prefix)
        .and_then(|tail| tail.strip_suffix(suffix))
        .is_some_and(|count| {
            !count.is_empty()
                && count.len() <= 9
                && count.bytes().all(|byte| byte.is_ascii_digit())
                && count.parse::<u32>().is_ok_and(|count| count > 0)
        })
}

fn recognized_guard(texts: &[String], buttons: &[String]) -> Option<&'static str> {
    if texts.len() != 2 || buttons.len() != 3 {
        return None;
    }
    let sets = [
        (
            "Claude is still working",
            "Quit anyway",
            "Wait for Claude",
            "Cancel",
            false,
        ),
        (
            "Claude はまだ作業中です",
            "このまま終了",
            "Claude を待つ",
            "キャンセル",
            true,
        ),
    ];
    for (message, quit, wait, cancel, japanese) in sets {
        if !texts.iter().any(|text| text == message)
            || [quit, wait, cancel].iter().any(|label| {
                buttons
                    .iter()
                    .filter(|button| button.as_str() == *label)
                    .count()
                    != 1
            })
        {
            continue;
        }
        let detail = texts.iter().find(|text| text.as_str() != message)?;
        let valid_detail = if japanese {
            session_count(
                detail,
                "Claude は ",
                " 件のセッションで作業中です。今終了すると、その作業が中断されます。",
            )
        } else {
            session_count(
                detail,
                "Claude is working in ",
                " session. Quitting now will interrupt that work.",
            ) || session_count(
                detail,
                "Claude is working in ",
                " sessions. Quitting now will interrupt that work.",
            )
        };
        if valid_detail {
            return Some(wait);
        }
    }
    None
}

// The adapter intentionally does not enumerate an app's ordinary accessibility
// descendants. It reads AXWindows, then only native modal windows/sheets; web
// areas, ordinary window titles, menus, chat/history and login UI stay unread.
struct Ax(CFTypeRef);

impl Drop for Ax {
    fn drop(&mut self) {
        // SAFETY: Ax owns exactly one retained Core Foundation object.
        unsafe { CFRelease(self.0) };
    }
}

impl Ax {
    fn application(pid: u32) -> Result<Self> {
        let pid = i32::try_from(pid).map_err(|_| unmanaged())?;
        // SAFETY: Public API creates an owned AX object for the known PID.
        let raw = unsafe { AXUIElementCreateApplication(pid) };
        if raw.is_null() {
            return Err(refuse("automatic_unknown_dialog"));
        }
        Self::bounded(raw)
    }

    fn bounded(raw: CFTypeRef) -> Result<Self> {
        let element = Self(raw);
        // SAFETY: caller supplies an owned, type-checked AX element. The SDK
        // specifies that timeouts are per object, not inherited by descendants.
        if unsafe { AXUIElementSetMessagingTimeout(element.0, 1.0) } != 0 {
            return Err(refuse("automatic_unknown_dialog"));
        }
        Ok(element)
    }

    fn check_pid(&self, pid: u32) -> Result<()> {
        let mut actual = 0;
        // SAFETY: self is a retained AX element; output points to a live pid_t.
        let status = unsafe { AXUIElementGetPid(self.0, &mut actual) };
        if status != 0 || u32::try_from(actual).ok() != Some(pid) {
            return Err(refuse("automatic_identity_changed"));
        }
        Ok(())
    }

    fn attribute(&self, name: &str) -> Result<Option<Self>> {
        let key = CFString::new(name);
        let mut value = std::ptr::null();
        // SAFETY: AX returns a retained CF object on success; this wrapper owns
        // it. Fixed attribute names are used only by the narrow modal reader.
        let status =
            unsafe { AXUIElementCopyAttributeValue(self.0, key.as_concrete_TypeRef(), &mut value) };
        match status {
            0 if !value.is_null() => Ok(Some(Self(value))),
            -25205 | -25212 => Ok(None), // unsupported attribute / no value
            _ => Err(refuse("automatic_unknown_dialog")),
        }
    }

    fn string(&self, name: &str) -> Result<Option<String>> {
        let Some(value) = self.attribute(name)? else {
            return Ok(None);
        };
        // SAFETY: Query type before interpreting a CF object as a CFString.
        if unsafe { CFGetTypeID(value.0) != CFStringGetTypeID() }
            || unsafe { CFStringGetLength(value.0.cast()) } > 512
        {
            return Err(refuse("automatic_unknown_dialog"));
        }
        // SAFETY: Type was checked; get-rule retains until conversion completes.
        Ok(Some(
            unsafe { CFString::wrap_under_get_rule(value.0.cast()) }.to_string(),
        ))
    }

    fn boolean(&self, name: &str) -> Result<Option<bool>> {
        let Some(value) = self.attribute(name)? else {
            return Ok(None);
        };
        // SAFETY: Query type before interpreting as a CFBoolean.
        if unsafe { CFGetTypeID(value.0) != CFBooleanGetTypeID() } {
            return Err(refuse("automatic_unknown_dialog"));
        }
        // SAFETY: Type checked above.
        Ok(Some(bool::from(unsafe {
            CFBoolean::wrap_under_get_rule(value.0.cast())
        })))
    }

    fn elements(&self, name: &str, limit: isize) -> Result<Vec<Self>> {
        let Some(value) = self.attribute(name)? else {
            return Ok(vec![]);
        };
        // SAFETY: Query type before using the CFArray API.
        if unsafe { CFGetTypeID(value.0) != CFArrayGetTypeID() } {
            return Err(refuse("automatic_unknown_dialog"));
        }
        // SAFETY: Confirmed CFArray, alive for this method.
        let count = unsafe { CFArrayGetCount(value.0.cast()) };
        if count < 0 || count > limit {
            return Err(refuse("automatic_unknown_dialog"));
        }
        let mut elements = Vec::new();
        for index in 0..count {
            // SAFETY: index is within the checked array. Validate AX type before
            // retaining so arbitrary CF values are never used as AX elements.
            let raw = unsafe { CFArrayGetValueAtIndex(value.0.cast(), index) };
            if raw.is_null() || unsafe { CFGetTypeID(raw) != AXUIElementGetTypeID() } {
                return Err(refuse("automatic_unknown_dialog"));
            }
            // SAFETY: Retain array's borrowed element for our own wrapper.
            elements.push(Self::bounded(unsafe { CFRetain(raw) })?);
        }
        Ok(elements)
    }

    fn press(&self) -> Result<()> {
        let action = CFString::new("AXPress");
        // SAFETY: Only invoked for the exact verified native Wait button. No
        // other action or app is ever passed to AXUIElementPerformAction.
        if unsafe { AXUIElementPerformAction(self.0, action.as_concrete_TypeRef()) } != 0 {
            return Err(refuse("automatic_quit_refused"));
        }
        Ok(())
    }
}

fn native_modals(pid: u32) -> Result<Vec<Ax>> {
    let app = Ax::application(pid)?;
    app.check_pid(pid)?;
    let mut modals = Vec::new();
    for window in app.elements("AXWindows", 16)? {
        window.check_pid(pid)?;
        if window.string("AXRole")?.as_deref() != Some("AXWindow") {
            return Err(refuse("automatic_unknown_dialog"));
        }
        if window.boolean("AXModal")? == Some(true)
            || matches!(
                window.string("AXSubrole")?.as_deref(),
                Some("AXDialog" | "AXSystemDialog")
            )
        {
            modals.push(window);
            continue;
        }
        for sheet in window.elements("AXSheets", 4)? {
            sheet.check_pid(pid)?;
            if sheet.string("AXRole")?.as_deref() != Some("AXSheet") {
                return Err(refuse("automatic_unknown_dialog"));
            }
            modals.push(sheet);
        }
    }
    Ok(modals)
}

#[derive(Default)]
struct ModalContents {
    nodes: usize,
    texts: Vec<String>,
    buttons: Vec<(String, Ax)>,
}

fn collect_modal(element: &Ax, pid: u32, depth: usize, contents: &mut ModalContents) -> Result<()> {
    if depth > 4 || contents.nodes >= 64 {
        return Err(refuse("automatic_unknown_dialog"));
    }
    contents.nodes += 1;
    element.check_pid(pid)?;
    let role = element
        .string("AXRole")?
        .ok_or_else(|| refuse("automatic_unknown_dialog"))?;
    match role.as_str() {
        "AXWindow" | "AXSheet" | "AXGroup" => {
            for child in element.elements("AXChildren", 16)? {
                collect_modal(&child, pid, depth + 1, contents)?;
            }
        }
        "AXStaticText" => {
            let text = element
                .string("AXValue")?
                .or(element.string("AXTitle")?)
                .ok_or_else(|| refuse("automatic_unknown_dialog"))?;
            contents.texts.push(text);
        }
        "AXButton" => {
            let title = element
                .string("AXTitle")?
                .ok_or_else(|| refuse("automatic_unknown_dialog"))?;
            // SAFETY: element is retained and was checked against the target PID.
            contents
                .buttons
                .push((title, Ax::bounded(unsafe { CFRetain(element.0) })?));
        }
        "AXImage" => {} // The native warning icon has no relevant text.
        _ => return Err(refuse("automatic_unknown_dialog")),
    }
    Ok(())
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef)
    -> bool;
    fn AXUIElementCreateApplication(pid: libc::pid_t) -> CFTypeRef;
    fn AXUIElementGetTypeID() -> core_foundation::base::CFTypeID;
    fn AXUIElementGetPid(element: CFTypeRef, pid: *mut libc::pid_t) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        name: core_foundation::string::CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(
        element: CFTypeRef,
        action: core_foundation::string::CFStringRef,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn absent_positive_guard_witness_always_refuses_before_quit() {
        assert_eq!(
            require_guard_readiness().unwrap_err().to_string(),
            "automatic_guard_unverified"
        );
    }

    #[test]
    fn only_complete_reviewed_native_guard_labels_are_accepted() {
        let en = strings(&[
            "Claude is still working",
            "Claude is working in 2 sessions. Quitting now will interrupt that work.",
        ]);
        let buttons = strings(&["Quit anyway", "Wait for Claude", "Cancel"]);
        assert_eq!(recognized_guard(&en, &buttons), Some("Wait for Claude"));
        let ja = strings(&[
            "Claude はまだ作業中です",
            "Claude は 1 件のセッションで作業中です。今終了すると、その作業が中断されます。",
        ]);
        assert_eq!(
            recognized_guard(
                &ja,
                &strings(&["このまま終了", "Claude を待つ", "キャンセル"])
            ),
            Some("Claude を待つ")
        );
        assert_eq!(
            recognized_guard(&en, &strings(&["Quit anyway", "Wait for Claude"])),
            None
        );
        assert_eq!(
            recognized_guard(
                &en,
                &strings(&["Quit anyway", "Wait for Claude", "Cancel", "Delete"])
            ),
            None
        );
        assert_eq!(
            recognized_guard(
                &en,
                &strings(&["このまま終了", "Claude を待つ", "キャンセル"])
            ),
            None
        );
        assert_eq!(
            recognized_guard(&strings(&["Claude is still working"]), &buttons),
            None
        );
        assert_eq!(
            recognized_guard(
                &strings(&[
                    "Delete all history?",
                    "Claude is working in 2 sessions. Quitting now will interrupt that work."
                ]),
                &buttons
            ),
            None
        );
        assert_eq!(
            recognized_guard(
                &strings(&[
                    "Claude is still working",
                    "Claude is working in x sessions. Quitting now will interrupt that work."
                ]),
                &buttons
            ),
            None
        );
    }

    fn proof() -> LaunchProof {
        LaunchProof {
            version: 1,
            pid: 123,
            launch_date_bits: 123_456_789.0_f64.to_bits(),
            desktop: "/test/Environment Data/desktop-data".into(),
            vendor_version: REVIEWED_VERSION.into(),
            asar_sha256: REVIEWED_ASAR.into(),
        }
    }

    #[test]
    fn proof_policy_rejects_unknown_builds_dates_and_unsafe_paths() {
        let good = proof();
        assert!(validate_proof_policy(&good).is_ok());
        let mut changed = good.clone();
        changed.vendor_version = "future".into();
        assert!(validate_proof_policy(&changed).is_err());
        changed = good.clone();
        changed.asar_sha256 = "different".into();
        assert!(validate_proof_policy(&changed).is_err());
        changed = good.clone();
        changed.launch_date_bits = f64::NAN.to_bits();
        assert!(validate_proof_policy(&changed).is_err());
        changed = good.clone();
        changed.desktop = "/test/../outside".into();
        assert!(validate_proof_policy(&changed).is_err());
        changed = good.clone();
        changed.desktop = "relative".into();
        assert!(validate_proof_policy(&changed).is_err());
        changed = good.clone();
        changed.desktop = "/test/new\nline".into();
        assert!(validate_proof_policy(&changed).is_err());
    }

    #[test]
    fn exact_launch_arguments_do_not_accept_sibling_or_hidden_flags() {
        let desktop = Path::new("/test/Environment Data/desktop-data");
        let expected = format!("{EXECUTABLE} --user-data-dir={}", desktop.display());
        assert!(exact_launch_args(&expected, desktop));
        assert!(!exact_launch_args(&(expected.clone() + "2"), desktop));
        assert!(!exact_launch_args(&(expected + " --hidden"), desktop));
        assert!(!exact_launch_args(EXECUTABLE, desktop));
    }

    #[test]
    fn private_launch_proof_rejects_links_and_unbounded_or_malformed_data() {
        use crate::platform::mock::MockPlatformProvider;
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::TempDir::new().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let provider = MockPlatformProvider::new(
            temp.path().join("desktop"),
            temp.path().join("cli"),
            temp.path().into(),
        );
        let good = proof();
        write_proof(&provider, &good).unwrap();
        assert_eq!(read_proof(&provider).unwrap(), good);
        let path = temp.path().join(PROOF_DIRECTORY).join(PROOF_FILE);
        let outside = temp.path().join("outside");
        fs::rename(&path, &outside).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(read_proof(&provider).is_err());
        fs::remove_file(&path).unwrap();
        fs::hard_link(&outside, &path).unwrap();
        assert!(read_proof(&provider).is_err());
        fs::remove_file(&path).unwrap();
        fs::remove_file(&outside).unwrap();
        write_proof(&provider, &good).unwrap();
        fs::write(&path, vec![b' '; 16_385]).unwrap();
        assert!(read_proof(&provider).is_err());
        fs::write(&path, br#"{"pid":123,"unexpected":true}"#).unwrap();
        assert!(read_proof(&provider).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_proof(&provider).is_err());
    }

    #[test]
    fn private_launch_proof_refuses_a_symlinked_storage_directory() {
        use crate::platform::mock::MockPlatformProvider;
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::TempDir::new().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let provider = MockPlatformProvider::new(
            temp.path().join("desktop"),
            temp.path().join("cli"),
            temp.path().into(),
        );
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, temp.path().join(PROOF_DIRECTORY)).unwrap();
        assert!(write_proof(&provider, &proof()).is_err());
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
    }
}
