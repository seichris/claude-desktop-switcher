#[test]
fn old_reset_never_becomes_zero() {
    let report = crate::usage::tests::report_from_synthetic_cache(
        72.0,
        1_800_000_000_000,
        1_800_000_001_000,
    );
    assert_eq!(report.windows[0].used_percent, Some(72.0));
    assert_eq!(report.state, crate::usage::UsageState::Stale);
}

use super::*;
use std::fs as disk;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;
const NOW: i64 = 1_800_000_000_000;
const ORG: &str = "11111111-2222-3333-4444-555555555555";
fn report_from_synthetic_cache(used: f64, reset: i64, now: i64) -> UsageReport {
    report(
        "example".into(),
        Observation {
            at: now,
            source: UsageSource::Cache,
            values: vec![("five_hour".into(), Some(used), Some(reset))],
        },
        now,
    )
}
fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
fn open(path: &Path) -> fs::Dir {
    fs::Dir::open(&path.canonicalize().unwrap()).unwrap()
}
fn fixture_history(path: &Path, org: &str, at: i64, used: f64) {
    disk::write(
        path.join("plan-usage-history.json"),
        serde_json::to_vec(&serde_json::json!({
            "version":2,"samples":[{"t":at,"org":org,"u":{"fh":used,"sd":9}}]
        }))
        .unwrap(),
    )
    .unwrap();
}
fn read(path: &Path) -> UsageReport {
    read_root(
        "example".into(),
        Ok(open(path)),
        NOW,
        Instant::now() + Duration::from_secs(3),
    )
}
#[test]
fn stale_values_and_missing_resets_are_explicit() {
    let temp = root();
    fixture_history(temp.path(), ORG, NOW - 8 * 60 * 60 * 1000, 65.0);
    let report = read(temp.path());
    assert_eq!(report.state, UsageState::Stale);
    assert_eq!(report.source, Some(UsageSource::History));
    assert_eq!(report.windows[0].used_percent, Some(65.0));
    assert_eq!(report.windows[0].resets_at_ms, None);
    assert_eq!(report.captured_at_ms, Some(NOW - 8 * 60 * 60 * 1000));
}
#[test]
fn freshness_boundary() {
    for (age, state) in [
        (FRESH_MS, UsageState::Fresh),
        (FRESH_MS + 1, UsageState::Stale),
    ] {
        let temp = root();
        fixture_history(temp.path(), ORG, NOW - age, 1.0);
        assert_eq!(read(temp.path()).state, state);
    }
}
#[test]
fn latest_organization_never_uses_older_organization() {
    let bytes = serde_json::to_vec(&serde_json::json!({"version":2,"samples":[
        {"t":NOW-1000,"org":ORG,"u":{"fh":8}},
        {"t":NOW,"org":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","u":{"fh":null}}
    ]}))
    .unwrap();
    let latest = history(&bytes, NOW).unwrap();
    assert_eq!(
        latest.org.as_deref(),
        Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
    );
    assert_eq!(latest.observation.unwrap().values[0].1, None);
}
#[test]
fn legacy_history_cannot_guess_an_organization() {
    let h = history(
        br#"{"version":1,"samples":[{"t":1800000000000,"fh":22,"sd":null}]}"#,
        NOW,
    )
    .unwrap();
    assert!(h.org.is_none());
    assert_eq!(h.observation.unwrap().values[0].1, Some(22.0));
}
#[test]
fn malformed_and_unknown_history_never_panics() {
    for data in [
        b"{".as_slice(),
        br#"{"version":99,"samples":[]}"#,
        br#"{"version":2,"samples":[{"t":1800000000000,"org":"../../outside","u":{"fh":5}}]}"#,
    ] {
        assert!(history(data, NOW).is_none());
    }
    let temp = root();
    assert_eq!(read(temp.path()).state, UsageState::Missing);
    disk::write(temp.path().join("plan-usage-history.json"), b"bad").unwrap();
    assert_eq!(read(temp.path()).state, UsageState::Unavailable);
}
#[test]
fn invalid_percentages_are_unknown_not_zero() {
    let obs=cached_observation(br#"{"five_hour":{"utilization":true},"seven_day":{"utilization":101},"seven_day_opus":{"utilization":-1}}"#,NOW).unwrap();
    assert_eq!(obs.values.len(), 2);
    assert!(obs.values.iter().all(|v| v.1.is_none()));
    assert_eq!(
        report("example".into(), obs, NOW).state,
        UsageState::Missing
    );
}
#[test]
fn explicit_cache_nulls_do_not_inherit_older_history() {
    let obs=cached_observation(br#"{"five_hour":{"utilization":0,"resets_at":null},"seven_day":null,"secret":"do-not-export"}"#,NOW).unwrap();
    let report = report("arbitrary_environment".into(), obs, NOW);
    assert_eq!(report.windows[0].used_percent, Some(0.0));
    assert_eq!(report.windows[1].used_percent, None);
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("do-not-export")
    );
}
#[test]
fn response_reset_times_are_parsed_without_inference() {
    let obs = cached_observation(
        br#"{"five_hour":{"utilization":72,"resets_at":"2027-01-15T08:00:00Z"},"seven_day":null}"#,
        NOW,
    )
    .unwrap();
    assert_eq!(obs.values[0].2, Some(1_800_000_000_000));
    let report = report("any".into(), obs, NOW + 1);
    assert_eq!(report.state, UsageState::Stale);
    assert_eq!(report.windows[0].used_percent, Some(72.0));
    assert_eq!(report.windows[0].state, WindowState::ResetPassed);
}
#[test]
fn usage_history_refuses_links_special_files_and_oversize() {
    let temp = root();
    let outside = root();
    fixture_history(outside.path(), ORG, NOW, 55.0);
    let file = temp.path().join("plan-usage-history.json");
    symlink(outside.path().join("plan-usage-history.json"), &file).unwrap();
    assert_eq!(read(temp.path()).state, UsageState::Unavailable);
    disk::remove_file(&file).unwrap();
    disk::hard_link(outside.path().join("plan-usage-history.json"), &file).unwrap();
    assert_eq!(read(temp.path()).state, UsageState::Unavailable);
    disk::remove_file(&file).unwrap();
    disk::write(&file, vec![0; HISTORY_LIMIT + 1]).unwrap();
    assert_eq!(read(temp.path()).state, UsageState::Unavailable);
    disk::remove_file(&file).unwrap();
    disk::create_dir(&file).unwrap();
    assert_eq!(read(temp.path()).state, UsageState::Unavailable);
}
#[test]
fn symlink_ancestors_are_refused() {
    let temp = root();
    let outside = root();
    symlink(outside.path(), temp.path().join("alias")).unwrap();
    assert!(fs::Dir::open(&temp.path().canonicalize().unwrap().join("alias")).is_err());
}
#[test]
fn all_discovered_names_use_fixed_roots_and_reads_never_write() {
    let temp = root();
    let base = temp.path().canonicalize().unwrap();
    let provider = Arc::new(crate::platform::mock::MockPlatformProvider::new(
        base.join("desktop"),
        base.join("cli"),
        base.join("csw"),
    ));
    let manager = ProfileManager::new(provider).unwrap();
    let names = ["sample_alpha", "日本語", "client-42"];
    for name in names {
        let path = base.join("csw/profiles").join(name);
        disk::create_dir_all(path.join("desktop-data")).unwrap();
        disk::write(path.join("profile.toml"), b"metadata only").unwrap();
        fixture_history(&path.join("desktop-data"), ORG, NOW, 30.0);
    }
    let profiles = base.join("csw/profiles");
    let before = disk::metadata(&profiles).unwrap();
    let (default, roots) = manager.usage_roots().unwrap();
    assert_eq!(default, base.join("desktop"));
    assert_eq!(roots, profiles);
    let reports = read_all_usage(&manager).unwrap();
    assert_eq!(reports.len(), 4);
    for name in names {
        assert!(reports.iter().any(|r| r.profile == name));
    }
    assert!(fs::stable(&before, &disk::metadata(profiles).unwrap()));
}
#[test]
fn same_organization_never_combines_profiles_and_newer_source_wins() {
    let a = root();
    let b = root();
    for (temp, used, at) in [(&a, 17, NOW), (&b, 83, NOW - 100)] {
        fixture_history(temp.path(), ORG, NOW - 1000, 1.0);
        let cache = temp.path().join("Cache/Cache_Data");
        disk::create_dir_all(&cache).unwrap();
        let json=serde_json::to_vec(&serde_json::json!({"five_hour":{"utilization":used,"resets_at":null},"seven_day":null})).unwrap();
        let url = format!("https://claude.ai/api/organizations/{ORG}/usage");
        disk::write(
            cache.join("1234567890abcdef_0"),
            simple::tests::fixture(url.as_bytes(), &json, at, true),
        )
        .unwrap();
    }
    assert_eq!(read(a.path()).windows[0].used_percent, Some(17.0));
    assert_eq!(read(b.path()).windows[0].used_percent, Some(83.0));
    fixture_history(a.path(), ORG, NOW, 42.0);
    // Replace the cache with an older reading: never attach its reset to newer history.
    let url = format!("https://claude.ai/api/organizations/{ORG}/usage");
    disk::write(
        a.path().join("Cache/Cache_Data/1234567890abcdef_0"),
        simple::tests::fixture(
            url.as_bytes(),
            br#"{"five_hour":{"utilization":1},"seven_day":null}"#,
            NOW - 10,
            false,
        ),
    )
    .unwrap();
    let report = read(a.path());
    assert_eq!(report.source, Some(UsageSource::History));
    assert_eq!(report.windows[0].used_percent, Some(42.0));
}
#[test]
fn fifo_input_does_not_block_and_refuses_special_files() {
    let temp = root();
    let path = temp.path().join("plan-usage-history.json");
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: this test owns the temporary path and passes a NUL-terminated name.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let start = Instant::now();
    assert_eq!(read(temp.path()).state, UsageState::Unavailable);
    assert!(start.elapsed() < Duration::from_secs(1));
}
