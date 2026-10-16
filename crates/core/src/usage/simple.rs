//! Chromium Simple Cache v5, independently implemented from Chromium's
//! net/disk_cache/simple/simple_entry_format.h and net/http/http_response_info.cc.
//! Only exact allowlisted usage keys get their bodies read. Unknown layouts fail closed.
use super::fs::{Dir, invalid, stable};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::time::Instant;

const INITIAL: u64 = 0xfcfb6d1ba7725c30;
const FINAL: u64 = 0xf4fa6f45970d41d8;
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_BODY: usize = 1024 * 1024;
const MAX_KEY: usize = 4096;
const MAX_META: usize = 64 * 1024;

pub(super) struct CachedBody {
    pub at: i64,
    pub body: Vec<u8>,
}

pub(super) fn latest(
    dir: &Dir,
    org: &str,
    now: i64,
    deadline: Instant,
) -> io::Result<Option<CachedBody>> {
    let cache = match dir
        .dir(OsStr::new("Cache"))
        .and_then(|d| d.dir(OsStr::new("Cache_Data")))
    {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let url = format!("https://claude.ai/api/organizations/{org}/usage");
    let mut best: Option<CachedBody> = None;
    let mut budget: usize = 16 * 1024 * 1024;
    for name in cache.names(50_000)? {
        if Instant::now() >= deadline {
            return Err(invalid());
        }
        if !cache_filename(&name) {
            continue;
        }
        let Ok(mut file) = cache.file(OsStr::new(&name)) else {
            continue;
        };
        let Ok(before) = file.metadata() else {
            continue;
        };
        if before.len() > MAX_FILE || before.len() < 72 {
            continue;
        }
        let Ok(key) = key(&mut file, before.len()) else {
            continue;
        };
        budget = budget.checked_sub(24 + key.len()).ok_or_else(invalid)?;
        if !matching_key(&key, &url) {
            continue;
        }
        let Ok(entry) = body(&mut file, &key, before.len(), now) else {
            continue;
        };
        if !stable(&before, &file.metadata()?) {
            continue;
        }
        if best.as_ref().is_none_or(|b| entry.at > b.at) {
            best = Some(entry);
        }
    }
    Ok(best)
}
fn cache_filename(name: &str) -> bool {
    name.len() == 18
        && name.ends_with("_0")
        && name.as_bytes()[..16]
            .iter()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn matching_key(key: &[u8], url: &str) -> bool {
    let Ok(key) = std::str::from_utf8(key) else {
        return false;
    };
    // Chromium's grammar is credential-key/upload-key/[isolation-key]URL.
    // Only GET-style upload key 0 is supported. A partition, when present,
    // starts with _dk_ and ends at the final unescaped space; resource URLs
    // cannot contain that delimiter. Never search for an embedded HTTPS URL.
    // See net/http/http_cache.cc:GetResourceURLFromHttpCacheKey.
    let resource = if let Some(rest) = key
        .strip_prefix("1/0/")
        .or_else(|| key.strip_prefix("0/0/"))
    {
        if rest.starts_with("_dk_") {
            let Some((partition, resource)) = rest.rsplit_once(' ') else {
                return false;
            };
            if partition.len() <= 4 || partition.bytes().any(|b| b.is_ascii_control() || b == 127) {
                return false;
            }
            resource
        } else {
            rest
        }
    } else {
        // Exact raw URL keys are harmless; unknown wrappers never match.
        key
    };
    resource == url || resource.strip_suffix("?skip_spend=1") == Some(url)
}

fn key(file: &mut File, len: u64) -> io::Result<Vec<u8>> {
    let header = read_at(file, 0, 24)?;
    let key_len = u32_at(&header, 12)? as usize;
    if u64_at(&header, 0)? != INITIAL
        || u32_at(&header, 8)? != 5
        || key_len == 0
        || key_len > MAX_KEY
        || 24 + key_len as u64 + 48 > len
    {
        return Err(invalid());
    }
    read_at(file, 24, key_len)
}
fn body(file: &mut File, key: &[u8], len: u64, now: i64) -> io::Result<CachedBody> {
    let eof0 = read_at(file, len - 24, 24)?;
    let flags0 = eof(&eof0)?;
    let meta_len = u32_at(&eof0, 16)? as usize;
    if meta_len > MAX_META {
        return Err(invalid());
    }
    let sha_len = if flags0 & 2 != 0 { 32 } else { 0 };
    let meta_start = len
        .checked_sub(24 + sha_len + meta_len as u64)
        .ok_or_else(invalid)?;
    let eof1_start = meta_start.checked_sub(24).ok_or_else(invalid)?;
    let body_start = 24 + key.len() as u64;
    let body_len = eof1_start.checked_sub(body_start).ok_or_else(invalid)?;
    if body_len == 0 || body_len > MAX_BODY as u64 {
        return Err(invalid());
    }
    let eof1 = read_at(file, eof1_start, 24)?;
    let flags1 = eof(&eof1)?;
    if flags1 & 2 != 0 {
        return Err(invalid());
    }
    if sha_len != 0 {
        let hash = read_at(file, len - 24 - 32, 32)?;
        if hash != Sha256::digest(key).as_slice() {
            return Err(invalid());
        }
    }
    let meta = read_at(file, meta_start, meta_len)?;
    if flags0 & 1 != 0 && crc32fast::hash(&meta) != u32_at(&eof0, 12)? {
        return Err(invalid());
    }
    let at = response_time(&meta, now)?;
    let encoded = read_at(file, body_start, body_len as usize)?;
    if flags1 & 1 != 0 && crc32fast::hash(&encoded) != u32_at(&eof1, 12)? {
        return Err(invalid());
    }
    Ok(CachedBody {
        at,
        body: decode(&encoded)?,
    })
}
fn eof(bytes: &[u8]) -> io::Result<u32> {
    let flags = u32_at(bytes, 8)?;
    if u64_at(bytes, 0)? != FINAL || flags & !3 != 0 {
        return Err(invalid());
    }
    Ok(flags)
}
fn response_time(meta: &[u8], now: i64) -> io::Result<i64> {
    if u32_at(meta, 0)? as usize != meta.len().checked_sub(4).ok_or_else(invalid)? {
        return Err(invalid());
    }
    let flags = u32_at(meta, 4)?;
    if flags & 255 != 3 || flags & (1 << 12) != 0 {
        return Err(invalid());
    }
    let start = if flags & (1 << 31) != 0 { 12 } else { 8 };
    // Chrome base::Time: microseconds since 1601-01-01. Do not read headers,
    // request cookies, authorization, or cache filesystem mtime as observation time.
    let raw = i64::from_le_bytes(
        slice(meta, start + 8, 8)?
            .try_into()
            .map_err(|_| invalid())?,
    );
    let at = raw
        .checked_div(1000)
        .and_then(|t| t.checked_sub(11_644_473_600_000))
        .ok_or_else(invalid)?;
    if !super::valid_time(at, now) {
        return Err(invalid());
    }
    Ok(at)
}
fn decode(encoded: &[u8]) -> io::Result<Vec<u8>> {
    if encoded.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut decoder =
            zstd::stream::read::Decoder::with_buffer(Cursor::new(encoded))?.single_frame();
        // Desktop HTTP streams declare an 8 MiB window even for small JSON.
        // Bound the decoder window separately from the 1 MiB output cap.
        decoder.window_log_max(23)?;
        let mut result = Vec::new();
        (&mut decoder)
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut result)?;
        if result.len() > MAX_BODY || decoder.finish().position() != encoded.len() as u64 {
            return Err(invalid());
        }
        Ok(result)
    } else if encoded.len() <= MAX_BODY
        && encoded.iter().find(|c| !c.is_ascii_whitespace()) == Some(&b'{')
    {
        Ok(encoded.to_vec())
    } else {
        Err(invalid())
    }
}
fn read_at(file: &mut File, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; len];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn slice(bytes: &[u8], at: usize, count: usize) -> io::Result<&[u8]> {
    bytes
        .get(at..at.checked_add(count).ok_or_else(invalid)?)
        .ok_or_else(invalid)
}
fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        slice(bytes, at, 4)?.try_into().map_err(|_| invalid())?,
    ))
}
fn u64_at(bytes: &[u8], at: usize) -> io::Result<u64> {
    Ok(u64::from_le_bytes(
        slice(bytes, at, 8)?.try_into().map_err(|_| invalid())?,
    ))
}

#[cfg(test)]
pub(super) mod tests;
