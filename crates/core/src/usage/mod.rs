//! Offline, read-only observations from each environment's Claude Desktop data.
//! No authentication files, credentials, network client, or profile mutations.
mod fs;
mod simple;

use crate::error::{CswError, Result};
use crate::profile::{ProfileManager, validate_profile_name};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsStr;
use std::io;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FRESH_MS: i64 = 30 * 60 * 1000;
const HISTORY_LIMIT: usize = 4 * 1024 * 1024;
const WINDOWS: [(&str, &str); 8] = [
    ("five_hour", "fh"),
    ("seven_day", "sd"),
    ("seven_day_opus", "so"),
    ("seven_day_sonnet", "sn"),
    ("seven_day_oauth_apps", "oa"),
    ("seven_day_cowork", "cw"),
    ("seven_day_omelette", "om"),
    ("omelette_promotional", "op"),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageState {
    Fresh,
    Stale,
    Missing,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Cache,
    History,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    Available,
    Missing,
    ResetPassed,
}
#[derive(Debug, Clone, Serialize)]
pub struct UsageWindow {
    pub key: String,
    pub used_percent: Option<f64>,
    pub resets_at_ms: Option<i64>,
    pub state: WindowState,
}
#[derive(Debug, Clone, Serialize)]
pub struct UsageReport {
    pub profile: String,
    pub state: UsageState,
    pub source: Option<UsageSource>,
    /// Original observation time, never the time CSW read the file.
    pub captured_at_ms: Option<i64>,
    pub windows: Vec<UsageWindow>,
}
impl UsageReport {
    fn empty(profile: String, state: UsageState) -> Self {
        Self {
            profile,
            state,
            source: None,
            captured_at_ms: None,
            windows: WINDOWS[..2]
                .iter()
                .map(|(key, _)| window(key, None, None, 0))
                .collect(),
        }
    }
}

/// Discover every environment dynamically. Roots come from the trusted platform
/// and CSW's fixed profile layout, never IPC arguments or profile.toml paths.
/// Raw filesystem/parser errors and identifiers are deliberately not exported.
pub fn read_all_usage(manager: &ProfileManager) -> Result<Vec<UsageReport>> {
    let (default_root, profiles_root) = manager.usage_roots()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CswError::Other("Usage clock unavailable".into()))?
        .as_millis();
    let now = i64::try_from(now).map_err(|_| CswError::Other("Usage clock unavailable".into()))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut reports = vec![read_root(
        "default".into(),
        fs::Dir::open(&default_root),
        now,
        deadline,
    )];
    let profiles = fs::Dir::open(&profiles_root)
        .map_err(|_| CswError::Other("Usage environments unavailable".into()))?;
    let mut names = profiles
        .names(50_000)
        .map_err(|_| CswError::Other("Usage environments unavailable".into()))?;
    names.sort();
    let mut count = 0;
    for name in names {
        if name == "default" || validate_profile_name(&name).is_err() {
            continue;
        }
        let Ok(profile) = profiles.dir(OsStr::new(&name)) else {
            continue;
        };
        // Presence only, do not deserialize notes, login information, or paths.
        if profile.file(OsStr::new("profile.toml")).is_err() {
            continue;
        }
        count += 1;
        if count > 1024 {
            return Err(CswError::Other("Too many usage environments".into()));
        }
        let root = profile.dir(OsStr::new("desktop-data"));
        reports.push(read_root(name, root, now, deadline));
    }
    Ok(reports)
}

fn read_root(
    profile: String,
    root: io::Result<fs::Dir>,
    now: i64,
    deadline: Instant,
) -> UsageReport {
    let dir = match root {
        Ok(dir) => dir,
        Err(e) => {
            return UsageReport::empty(
                profile,
                if e.kind() == io::ErrorKind::NotFound {
                    UsageState::Missing
                } else {
                    UsageState::Unavailable
                },
            );
        }
    };
    if Instant::now() >= deadline {
        return UsageReport::empty(profile, UsageState::Unavailable);
    }
    let bytes = match dir.read("plan-usage-history.json", HISTORY_LIMIT) {
        Ok(bytes) => bytes,
        Err(e) => {
            return UsageReport::empty(
                profile,
                if e.kind() == io::ErrorKind::NotFound {
                    UsageState::Missing
                } else {
                    UsageState::Unavailable
                },
            );
        }
    };
    let Some(history) = history(&bytes, now) else {
        return UsageReport::empty(profile, UsageState::Unavailable);
    };
    let mut observation = history.observation;
    if let Some(org) = history.org {
        // Organization ID is only a key within this already-open profile root.
        // It is never an account identity and never joins data across roots.
        let profile_deadline = deadline.min(Instant::now() + Duration::from_secs(3));
        if let Ok(Some(cached)) = simple::latest(&dir, &org, now, profile_deadline)
            && observation.as_ref().is_none_or(|h| cached.at >= h.at)
            && let Some(newer) = cached_observation(&cached.body, cached.at)
        {
            observation = Some(newer);
        }
    }
    match observation {
        Some(observation) => report(profile, observation, now),
        None => UsageReport::empty(profile, UsageState::Missing),
    }
}

struct Observation {
    at: i64,
    source: UsageSource,
    values: Vec<(String, Option<f64>, Option<i64>)>,
}
struct History {
    org: Option<String>,
    observation: Option<Observation>,
}
#[derive(Deserialize)]
struct HistoryFile {
    version: u32,
    samples: Vec<HistorySample>,
}
#[derive(Deserialize)]
struct HistorySample {
    t: Value,
    #[serde(default)]
    org: Option<String>,
    #[serde(default)]
    u: Value,
    #[serde(default)]
    fh: Value,
    #[serde(default)]
    sd: Value,
}
fn history(bytes: &[u8], now: i64) -> Option<History> {
    let history: HistoryFile = serde_json::from_slice(bytes).ok()?;
    if !matches!(history.version, 1 | 2) || history.samples.len() > 20_000 {
        return None;
    }
    let sample = history
        .samples
        .iter()
        .filter_map(|s| s.t.as_i64().filter(|t| valid_time(*t, now)).map(|t| (t, s)))
        .max_by_key(|(t, _)| *t);
    let Some((at, sample)) = sample else {
        return Some(History {
            org: None,
            observation: None,
        });
    };
    let org = sample.org.as_deref().filter(|s| uuid(s)).map(str::to_owned);
    if history.version == 2 && org.is_none() {
        return None;
    }
    let values = WINDOWS
        .iter()
        .enumerate()
        .filter_map(|(i, (key, short))| {
            let value = if history.version == 1 {
                match i {
                    0 => &sample.fh,
                    1 => &sample.sd,
                    _ => &Value::Null,
                }
            } else {
                sample.u.get(short).unwrap_or(&Value::Null)
            };
            let used = percent(value);
            (i < 2 || used.is_some()).then(|| (key.to_string(), used, None))
        })
        .collect();
    Some(History {
        org: if history.version == 2 { org } else { None },
        observation: Some(Observation {
            at,
            source: UsageSource::History,
            values,
        }),
    })
}
fn cached_observation(body: &[u8], at: i64) -> Option<Observation> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let object = json.as_object()?;
    if !object.contains_key("five_hour") && !object.contains_key("seven_day") {
        return None;
    }
    let values = WINDOWS
        .iter()
        .enumerate()
        .filter_map(|(i, (key, _))| {
            let data = object.get(*key);
            let used = data.and_then(|d| d.get("utilization")).and_then(percent);
            let reset = data
                .and_then(|d| d.get("resets_at"))
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 40)
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp_millis())
                .filter(|t| (1_577_836_800_000..253_402_300_800_000).contains(t));
            (i < 2 || used.is_some() || reset.is_some()).then(|| (key.to_string(), used, reset))
        })
        .collect();
    Some(Observation {
        at,
        source: UsageSource::Cache,
        values,
    })
}
fn percent(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
}
fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                *c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
fn valid_time(at: i64, now: i64) -> bool {
    (1_577_836_800_000..=now.saturating_add(60_000)).contains(&at)
}
fn window(key: &str, used: Option<f64>, reset: Option<i64>, now: i64) -> UsageWindow {
    UsageWindow {
        key: key.into(),
        used_percent: used,
        resets_at_ms: reset,
        state: if reset.is_some_and(|t| t <= now) {
            WindowState::ResetPassed
        } else if used.is_some() {
            WindowState::Available
        } else {
            WindowState::Missing
        },
    }
}
fn report(profile: String, observation: Observation, now: i64) -> UsageReport {
    let windows: Vec<_> = observation
        .values
        .iter()
        .map(|(key, used, reset)| window(key, *used, *reset, now))
        .collect();
    let state = if !windows
        .iter()
        .any(|w| w.used_percent.is_some() || w.resets_at_ms.is_some())
    {
        UsageState::Missing
    } else if now.saturating_sub(observation.at) > FRESH_MS
        || windows.iter().any(|w| w.state == WindowState::ResetPassed)
    {
        UsageState::Stale
    } else {
        UsageState::Fresh
    };
    UsageReport {
        profile,
        state,
        source: Some(observation.source),
        captured_at_ms: Some(observation.at),
        windows,
    }
}

#[cfg(test)]
mod tests;
