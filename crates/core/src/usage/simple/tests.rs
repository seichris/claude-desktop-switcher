use super::*;

const NOW: i64 = 1_800_000_000_000;
const ORG: &str = "11111111-2222-3333-4444-555555555555";
fn eof_bytes(flags: u32, crc: u32, size: u32) -> Vec<u8> {
    [
        FINAL.to_le_bytes().as_slice(),
        &flags.to_le_bytes(),
        &crc.to_le_bytes(),
        &size.to_le_bytes(),
        &0_u32.to_le_bytes(),
    ]
    .concat()
}
pub(in crate::usage) fn fixture(key: &[u8], json: &[u8], at: i64, compressed: bool) -> Vec<u8> {
    let body = if compressed {
        zstd::stream::encode_all(json, 1).unwrap()
    } else {
        json.to_vec()
    };
    let raw = (at + 11_644_473_600_000) * 1000;
    let mut meta = [
        0_u32.to_le_bytes().as_slice(),
        &3_u32.to_le_bytes(),
        &raw.to_le_bytes(),
        &raw.to_le_bytes(),
    ]
    .concat();
    let size = (meta.len() - 4) as u32;
    meta[..4].copy_from_slice(&size.to_le_bytes());
    let mut out = [
        INITIAL.to_le_bytes().as_slice(),
        &5_u32.to_le_bytes(),
        &(key.len() as u32).to_le_bytes(),
        &0_u32.to_le_bytes(),
        &0_u32.to_le_bytes(),
        key,
    ]
    .concat();
    out.extend(&body);
    out.extend(eof_bytes(1, crc32fast::hash(&body), 0));
    out.extend(&meta);
    out.extend(Sha256::digest(key));
    out.extend(eof_bytes(3, crc32fast::hash(&meta), meta.len() as u32));
    out
}
fn url() -> String {
    format!("https://claude.ai/api/organizations/{ORG}/usage")
}
fn parse(bytes: &[u8]) -> io::Result<CachedBody> {
    let mut file = tempfile::tempfile().unwrap();
    use std::io::Write;
    file.write_all(bytes).unwrap();
    let key = key(&mut file, bytes.len() as u64)?;
    body(&mut file, &key, bytes.len() as u64, NOW)
}
#[test]
fn plain_and_zstandard_cache_with_crc_and_sha() {
    let json = br#"{"five_hour":{"utilization":28,"resets_at":null},"seven_day":null}"#;
    for compressed in [false, true] {
        let entry = parse(&fixture(url().as_bytes(), json, NOW - 100, compressed)).unwrap();
        assert_eq!(entry.at, NOW - 100);
        assert_eq!(entry.body, json);
    }
}
#[test]
fn exact_usage_key_only() {
    let url = url();
    for prefix in ["", "1/0/", "1/0/_dk_https://claude.ai "] {
        assert!(matching_key(format!("{prefix}{url}").as_bytes(), &url));
        assert!(matching_key(
            format!("{prefix}{url}?skip_spend=1").as_bytes(),
            &url
        ));
    }
    for suffix in [
        "?token=secret",
        "/anything",
        "?skip_spend=1&extra=1",
        "#fragment",
        "?skip_spend=0",
    ] {
        assert!(!matching_key(format!("{url}{suffix}").as_bytes(), &url));
    }
    assert!(!matching_key(
        b"https://claude.ai/api/organizations/other/usage",
        &url
    ));
    assert!(!matching_key(b"https://other.example/api/usage", &url));
}
#[test]
fn corruption_truncation_and_huge_lengths_are_rejected() {
    let bytes = fixture(url().as_bytes(), br#"{"five_hour":null}"#, NOW, false);
    for len in [0, 8, 23, 24, bytes.len() - 1] {
        assert!(parse(&bytes[..len]).is_err());
    }
    for at in [
        0,
        8,
        12,
        24,
        24 + url().len(),
        bytes.len() - 25,
        bytes.len() - 12,
    ] {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 0xff;
        assert!(parse(&corrupt).is_err(), "offset {at}");
    }
}
#[test]
fn response_timestamp_comes_from_metadata_and_future_is_rejected() {
    assert!(
        parse(&fixture(
            url().as_bytes(),
            br#"{"five_hour":null}"#,
            NOW + 60_001,
            false
        ))
        .is_err()
    );
    let mut meta = vec![0; 24];
    meta[0..4].copy_from_slice(&20_u32.to_le_bytes());
    meta[4..8].copy_from_slice(&(3_u32 | (1 << 12)).to_le_bytes());
    assert!(response_time(&meta, NOW).is_err());
}
#[test]
fn decompression_bombs_and_trailing_frames_are_rejected() {
    let oversized = zstd::stream::encode_all(vec![b' '; MAX_BODY + 1].as_slice(), 1).unwrap();
    assert!(decode(&oversized).is_err());
    let frame = zstd::stream::encode_all(b"{}".as_slice(), 1).unwrap();
    let mut trailing = frame.clone();
    trailing.extend(&frame);
    assert!(decode(&trailing).is_err());
    trailing = frame;
    trailing.push(0);
    assert!(decode(&trailing).is_err());
    assert!(decode(b"not json").is_err());
}
#[test]
fn wrong_files_and_organizations_are_not_read_as_usage() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let cache = root.join("Cache/Cache_Data");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("aaaaaaaaaaaaaaaa_0"),
        fixture(b"https://other.example/private", b"secret", NOW, false),
    )
    .unwrap();
    std::fs::write(
        cache.join("bbbbbbbbbbbbbbbb_0"),
        fixture(url().as_bytes(), br#"{"five_hour":null}"#, NOW, false),
    )
    .unwrap();
    let dir = Dir::open(&root).unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    assert!(
        latest(&dir, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", NOW, deadline)
            .unwrap()
            .is_none()
    );
    assert_eq!(latest(&dir, ORG, NOW, deadline).unwrap().unwrap().at, NOW);
}
#[test]
fn cache_symlink_and_deadline_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("Cache")).unwrap();
    let dir = Dir::open(&root).unwrap();
    assert!(latest(&dir, ORG, NOW, Instant::now()).is_err());
}
#[test]
fn streaming_http_frames_allow_bounded_eight_megabyte_windows() {
    use std::io::Write;
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    encoder.window_log(23).unwrap();
    encoder.write_all(b"{\"five_hour\":null}").unwrap();
    let frame = encoder.finish().unwrap();
    assert_eq!(decode(&frame).unwrap(), b"{\"five_hour\":null}");
}
#[test]
fn larger_than_supported_compression_windows_are_refused() {
    use std::io::Write;
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    encoder.window_log(24).unwrap();
    encoder.write_all(b"{}").unwrap();
    assert!(decode(&encoder.finish().unwrap()).is_err());
}
