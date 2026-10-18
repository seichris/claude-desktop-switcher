//! Explicit one-case native test. Never selects or sends a quit to Claude.
//! CSW_NATIVE_GUARD_CASE=sheet-en cargo test -p csw-core native_guard_fixture -- --ignored --nocapture
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::process::Child;

/// Serializes visible tests and refuses rapid retries even across invocations.
struct NativePermit(File);

impl NativePermit {
    fn acquire() -> Self {
        let directory = dirs::cache_dir().unwrap().join("csw-native-fixture-tests");
        match fs::create_dir(&directory) {
            Ok(()) => fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap(),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists),
        }
        let metadata = fs::symlink_metadata(&directory).unwrap();
        assert!(metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o777, 0o700);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(directory.join("visible-test.lock"))
            .unwrap();
        Self::claim_and_stamp(file, |file| {
            // SAFETY: live owned descriptor, advisory nonblocking test lock.
            assert_eq!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                0,
                "another visible fixture is running; no dialog opened"
            );
        })
    }

    fn claim_and_stamp(mut file: File, claim: impl FnOnce(&File)) -> Self {
        let metadata = file.metadata().unwrap();
        assert!(metadata.is_file() && metadata.nlink() == 1);
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o777, 0o600);
        claim(&file);
        // Another invocation may have stamped this file while we were delayed
        // before flock. Only metadata observed while holding the lock is fresh.
        let metadata = file.metadata().unwrap();
        assert!(metadata.len() == 0 || metadata.len() == 8);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if metadata.len() == 8 {
            let mut previous = [0; 8];
            file.read_exact(&mut previous).unwrap();
            assert!(
                now.checked_sub(u64::from_le_bytes(previous))
                    .is_some_and(|age| age >= 30),
                "visible test cooldown: wait 30 seconds; no dialog opened"
            );
        }
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&now.to_le_bytes()).unwrap();
        file.sync_all().unwrap();
        Self(file)
    }
}

#[test]
fn cooldown_rechecks_metadata_after_claim() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    // Deterministically model another invocation stamping the initially empty
    // file while this invocation is delayed before it acquires the lock.
    let outcome = std::panic::catch_unwind(|| {
        NativePermit::claim_and_stamp(file, |_| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            fs::write(&path, now.to_le_bytes()).unwrap();
        })
    });
    let Err(error) = outcome else {
        panic!("stale pre-lock metadata bypassed the cooldown")
    };
    let message = error
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied());
    assert!(message.is_some_and(|message| message.contains("visible test cooldown")));
}

impl Drop for NativePermit {
    fn drop(&mut self) {
        // SAFETY: release only our own live descriptor's test lock.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

struct Fixture {
    child: Child,
    directory: tempfile::TempDir,
}

impl Fixture {
    fn mark(&self, name: &str) {
        fs::write(self.directory.path().join(name), []).unwrap();
    }
    fn has(&self, name: &str) -> bool {
        self.directory.path().join(name).exists()
    }
    fn wait_for(&mut self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.has(name) {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "fixture exited before {name}"
            );
            assert!(
                Instant::now() < deadline,
                "fixture did not reach {name}; no retries"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    fn modal(&mut self) -> ModalContents {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut modals = loop {
            let modals = native_modals(self.child.id()).unwrap();
            if !modals.is_empty() || Instant::now() >= deadline {
                break modals;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(
            modals.len(),
            1,
            "expected one native fixture sheet; no retries"
        );
        let mut contents = ModalContents::default();
        collect_modal(&modals.pop().unwrap(), self.child.id(), 0, &mut contents).unwrap();
        contents
    }
    fn events(&self) -> Vec<String> {
        fs::read_to_string(self.directory.path().join("events"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn finish(&mut self) {
        self.mark("shutdown");
        assert!(self.child.wait().unwrap().success());
        let events = self.events();
        assert_eq!(
            events.iter().filter(|event| *event == "presented").count(),
            1
        );
        assert_eq!(
            events.iter().filter(|event| *event == "finished").count(),
            1
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Best effort during assertion unwinding; the app also self-exits after
        // 20 seconds. Never signal a process, and never relaunch a failed test.
        let _ = fs::write(self.directory.path().join("shutdown"), []);
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "internal no-window subprocess probe; invoked only by native_permission_fixture"]
fn native_permission_probe() {
    let directory = PathBuf::from(
        std::env::var("CSW_PERMISSION_PROBE_DIRECTORY")
            .expect("internal permission probe requires its private directory"),
    );
    // The freshly launched probe has its own app identity and asks for no grant.
    // Do not modify TCC or revoke permissions from any real application.
    assert!(
        !trusted(),
        "probe inherited a grant; missing permission was not tested"
    );
    let provider = crate::platform::mock::MockPlatformProvider::new(
        directory.join("desktop"),
        directory.join("cli"),
        directory.clone(),
    );
    let app = NSRunningApplication::currentApplication();
    let mut quit = SafeQuit {
        provider: &provider,
        app,
        proof: LaunchProof {
            version: 1,
            pid: std::process::id(),
            launch_date_bits: 1,
            desktop: directory.join("desktop"),
            vendor_version: REVIEWED_VERSION.into(),
            asar_sha256: REVIEWED_ASAR.into(),
        },
        vendor_stamp: vec![],
        requested: Some(Instant::now()),
        waiting: true,
    };
    assert_eq!(
        quit.poll().unwrap_err().to_string(),
        "automatic_accessibility_required"
    );
    assert_eq!(
        quit.request().unwrap_err().to_string(),
        "automatic_guard_unverified"
    );
    fs::write(
        directory.join("passed"),
        b"untrusted wait-state refused before UI or file reads",
    )
    .unwrap();
}

#[test]
#[ignore = "starts a no-window disposable app with no permission prompt"]
fn native_permission_fixture() {
    assert_eq!(
        std::env::var("CSW_NATIVE_PERMISSION_TEST").as_deref(),
        Ok("run-once"),
        "explicit permission test opt-in required; no app opened"
    );
    let directory = tempfile::tempdir().unwrap();
    let application = directory.path().join("CSW Permission Probe.app");
    let macos = application.join("Contents/MacOS");
    fs::create_dir_all(&macos).unwrap();
    fs::copy(
        std::env::current_exe().unwrap(),
        macos.join("CSWPermissionProbe"),
    )
    .unwrap();
    fs::write(
        application.join("Contents/Info.plist"),
        br#"<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.seichris.csw.permission-test-probe</string>
<key>CFBundleExecutable</key><string>CSWPermissionProbe</string>
<key>CFBundleName</key><string>CSW Permission Probe</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSUIElement</key><true/></dict></plist>"#,
    )
    .unwrap();
    let result = Command::new("/usr/bin/open")
        .args(["-g", "-n", "-W", "--env"])
        .arg(format!(
            "CSW_PERMISSION_PROBE_DIRECTORY={}",
            directory.path().display()
        ))
        .arg(&application)
        .args([
            "--args",
            "--ignored",
            "--exact",
            "platform::automatic::native_tests::native_permission_probe",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "permission probe could not launch");
    assert_eq!(
        fs::read_to_string(directory.path().join("passed")).unwrap(),
        "untrusted wait-state refused before UI or file reads"
    );
    println!(
        "PASS no-window native probe: absent permission refuses wait-state polling; no permission changed"
    );
}

#[test]
#[ignore = "explicit CSW_NATIVE_GUARD_CASE required; opens ONE disposable test sheet"]
fn native_guard_fixture() {
    let case = std::env::var("CSW_NATIVE_GUARD_CASE")
        .expect("explicit test case required; no dialog opened");
    assert!(
        ["sheet-en", "sheet-ja", "unknown"].contains(&case.as_str()),
        "unknown case; no dialog opened"
    );
    assert!(
        trusted(),
        "requires an existing Accessibility grant; no prompt or dialog opened"
    );
    assert!(
        require_guard_readiness().is_err(),
        "production quarantine must remain"
    );
    let build = tempfile::tempdir().unwrap();
    let application = build.path().join("CSW Test Fixture.app");
    let macos = application.join("Contents/MacOS");
    fs::create_dir_all(&macos).unwrap();
    fs::write(
        application.join("Contents/Info.plist"),
        br#"<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.seichris.csw.native-test-fixture</string>
<key>CFBundleExecutable</key><string>CSWTestFixture</string>
<key>CFBundleName</key><string>CSW TEST FIXTURE</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSUIElement</key><true/></dict></plist>"#,
    )
    .unwrap();
    let executable = macos.join("CSWTestFixture");
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/automatic-switch");
    let compile = Command::new("xcrun")
        .arg("swiftc")
        .arg("-warnings-as-errors")
        .arg(scripts.join("FixtureLifecycle.swift"))
        .arg(scripts.join("native-guard-fixture.swift"))
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let _permit = NativePermit::acquire();
    let directory = tempfile::tempdir().unwrap();
    let child = Command::new(&executable)
        .arg("--one-disposable-dialog")
        .arg(&case)
        .arg(directory.path())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut fixture = Fixture { child, directory };
    fixture.wait_for("ready");
    let contents = fixture.modal();
    let labels: Vec<_> = contents
        .buttons
        .iter()
        .map(|(label, _)| label.clone())
        .collect();
    if case == "unknown" {
        assert_eq!(recognized_guard(&contents.texts, &labels), None);
        // Leave this single non-actionable fixture briefly visible for manual
        // verification of its TEST branding; never reopen or advance a case.
        println!("Unknown fixture is visible for 8 seconds; no button will be pressed");
        std::thread::sleep(Duration::from_secs(8));
        fixture.finish();
        assert!(
            !fixture
                .events()
                .iter()
                .any(|event| event.starts_with("pressed-"))
        );
        println!("PASS native unknown sheet: no action, one presentation, one shutdown");
        return;
    }
    let expected = if case == "sheet-ja" {
        "Claude を待つ"
    } else {
        "Wait for Claude"
    };
    assert_eq!(recognized_guard(&contents.texts, &labels), Some(expected));
    let (_, button) = contents
        .buttons
        .iter()
        .find(|(label, _)| label == expected)
        .unwrap();
    assert_eq!(button.boolean("AXEnabled").unwrap(), Some(true));
    assert!(
        button.check_pid(std::process::id()).is_err(),
        "wrong PID must refuse"
    );
    button.check_pid(fixture.child.id()).unwrap();
    button.press().unwrap();
    fixture.wait_for("pressed-1");
    for completed in ["turn-done", "input-done"] {
        fixture.mark(completed);
        std::thread::sleep(Duration::from_millis(200));
        assert!(!fixture.has("safe-exit"));
        assert!(fixture.child.try_wait().unwrap().is_none());
    }
    fixture.mark("background-done");
    fixture.wait_for("safe-exit");
    fixture.finish();
    assert_eq!(
        fixture
            .events()
            .iter()
            .filter(|event| event.starts_with("pressed-"))
            .count(),
        1
    );
    assert!(
        button.press().is_err(),
        "exited fixture cannot receive a second action"
    );
    println!(
        "PASS native {case}: exact Wait, pending fake work, one presentation/shutdown, stale element refused"
    );
}
