//! Bounds cache rescans even when multiple frontend requests arrive together.

use std::time::{Duration, Instant};

const READ_INTERVAL: Duration = Duration::from_secs(30);

pub enum ReadDecision<T> {
    Cached(Result<T, &'static str>),
    Start,
    Pending,
}

pub struct ReadCache<T> {
    reading: bool,
    snapshot: Option<(Instant, Result<T, &'static str>)>,
}

impl<T: Clone> ReadCache<T> {
    pub fn new() -> Self {
        Self {
            reading: false,
            snapshot: None,
        }
    }

    pub fn begin(&mut self, now: Instant) -> ReadDecision<T> {
        if let Some((checked, value)) = &self.snapshot
            && (self.reading || now.saturating_duration_since(*checked) < READ_INTERVAL)
        {
            return ReadDecision::Cached(value.clone());
        }
        if self.reading {
            return ReadDecision::Pending;
        }
        self.reading = true;
        ReadDecision::Start
    }

    pub fn finish(&mut self, now: Instant, value: Result<T, &'static str>) {
        self.snapshot = Some((now, value));
        self.reading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_requests_start_one_scan_and_then_reuse_snapshot() {
        let mut cache = ReadCache::new();
        let now = Instant::now();
        assert!(matches!(cache.begin(now), ReadDecision::Start));
        assert!(matches!(cache.begin(now), ReadDecision::Pending));
        cache.finish(now, Ok(vec![42]));
        assert!(matches!(cache.begin(now), ReadDecision::Cached(Ok(v)) if v == vec![42]));
        assert!(matches!(
            cache.begin(now + READ_INTERVAL),
            ReadDecision::Start
        ));
        assert!(
            matches!(cache.begin(now + READ_INTERVAL), ReadDecision::Cached(Ok(v)) if v == vec![42])
        );
    }

    #[test]
    fn failed_reads_are_rate_limited_too() {
        let mut cache = ReadCache::<Vec<u8>>::new();
        let now = Instant::now();
        assert!(matches!(cache.begin(now), ReadDecision::Start));
        cache.finish(now, Err("usage_unavailable"));
        assert!(matches!(
            cache.begin(now + Duration::from_secs(29)),
            ReadDecision::Cached(Err("usage_unavailable"))
        ));
        assert!(matches!(
            cache.begin(now + READ_INTERVAL),
            ReadDecision::Start
        ));
    }
}
