use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use super::{time_of, Rec, MICROS_PER_SECOND};

/// Past this many records the oldest seconds are dropped - a backstop for when the writes
/// fall behind. Five times what my-logger-server keeps: the time to wait is a setting here,
/// and telemetry comes by the thousand a second.
const MAX_PENDING_RECORDS: usize = 1_000_000;

/// A time further than this ahead of the server's is taken for a wrong one
const MAX_TIME_AHEAD_MICROS: i64 = 24 * 60 * 60 * MICROS_PER_SECOND;

struct PendingBucket {
    created: Instant,
    records: Vec<Rec>,
}

/// The records of one second of a product scope which have waited long enough: in time
/// order, as they are stored
pub struct ReadyBucket {
    pub scope: Arc<str>,
    /// Unix seconds
    pub second: i64,
    pub records: Vec<Rec>,
}

/// The records waiting to be written, by product scope and second. A second waits from its
/// first record, so the records of it which come a little late are written with it.
pub struct PendingMetrics {
    buckets: BTreeMap<(Arc<str>, i64), PendingBucket>,
    /// The seconds taken to be written and not written yet. A search still finds them
    /// here: there is no moment a record is neither in memory nor in the files.
    being_written: Vec<Arc<ReadyBucket>>,
    /// The records of `buckets`
    records: usize,
    max_records: usize,
}

impl Default for PendingMetrics {
    fn default() -> Self {
        Self {
            buckets: BTreeMap::new(),
            being_written: Vec::new(),
            records: 0,
            max_records: MAX_PENDING_RECORDS,
        }
    }
}

impl PendingMetrics {
    /// Returns how many records were dropped to keep the buffer bounded
    pub fn add(
        &mut self,
        scope: &Arc<str>,
        records: impl IntoIterator<Item = Rec>,
        now: Instant,
        now_micros: i64,
    ) -> usize {
        for mut record in records {
            put_in_time(&mut record, now_micros);

            let key = (scope.clone(), time_of(&record).div_euclid(MICROS_PER_SECOND));

            self.buckets
                .entry(key)
                .or_insert_with(|| PendingBucket {
                    created: now,
                    records: Vec::new(),
                })
                .records
                .push(record);

            self.records += 1;
        }

        let mut dropped = 0;

        while self.records > self.max_records {
            let oldest = self
                .buckets
                .iter()
                .min_by_key(|(_, bucket)| bucket.created)
                .map(|(key, _)| key.clone())
                .unwrap();

            let bucket = self.buckets.remove(&oldest).unwrap();
            self.records -= bucket.records.len();
            dropped += bucket.records.len();
        }

        dropped
    }

    /// Takes the seconds which have waited `window` - or all of them - to be written.
    /// `prepare` is given them to do what is done to the records at the last moment before
    /// they are written. They stay to be found by a search until [`Self::written`].
    pub fn take_ready(
        &mut self,
        now: Instant,
        window: Duration,
        all: bool,
        prepare: impl FnOnce(&mut [ReadyBucket]),
    ) -> Vec<Arc<ReadyBucket>> {
        let keys: Vec<(Arc<str>, i64)> = self
            .buckets
            .iter()
            .filter(|(_, bucket)| all || now.duration_since(bucket.created) >= window)
            .map(|(key, _)| key.clone())
            .collect();

        let mut ready: Vec<ReadyBucket> = keys
            .into_iter()
            .map(|key| {
                let mut bucket = self.buckets.remove(&key).unwrap();
                self.records -= bucket.records.len();

                // As they are stored: in time order
                bucket.records.sort_by_key(time_of);

                let (scope, second) = key;

                ReadyBucket {
                    scope,
                    second,
                    records: bucket.records,
                }
            })
            .collect();

        prepare(&mut ready);

        let ready: Vec<Arc<ReadyBucket>> = ready.into_iter().map(Arc::new).collect();
        self.being_written.extend(ready.iter().cloned());

        ready
    }

    /// The seconds are in the files - or are dropped: they are let go of
    pub fn written(&mut self, written: &[Arc<ReadyBucket>]) {
        self.being_written
            .retain(|bucket| !written.iter().any(|written| Arc::ptr_eq(written, bucket)));
    }

    /// The records which are not written yet
    pub fn len(&self) -> usize {
        self.records
            + self
                .being_written
                .iter()
                .map(|bucket| bucket.records.len())
                .sum::<usize>()
    }

    /// The scopes which have records waiting
    pub fn scopes(&self) -> impl Iterator<Item = &Arc<str>> {
        self.buckets
            .keys()
            .map(|(scope, _)| scope)
            .chain(self.being_written.iter().map(|bucket| &bucket.scope))
    }

    /// The records in `from ..= to` which match - of the scope, or of every scope - among
    /// the ones which wait and the ones which are being written
    pub fn search(
        &self,
        scope: Option<&str>,
        from: i64,
        to: i64,
        matches: impl Fn(&Rec) -> bool,
    ) -> Vec<Rec> {
        let waiting = self
            .buckets
            .iter()
            .map(|((scope, second), bucket)| (scope, *second, &bucket.records));

        let being_written = self
            .being_written
            .iter()
            .map(|bucket| (&bucket.scope, bucket.second, &bucket.records));

        let mut result = Vec::new();

        for (bucket_scope, second, records) in waiting.chain(being_written) {
            let second_start = second * MICROS_PER_SECOND;

            if scope.map_or(false, |scope| scope != &**bucket_scope)
                || second_start > to
                || second_start + MICROS_PER_SECOND <= from
            {
                continue;
            }

            for record in records {
                let time = time_of(record);

                if time >= from && time <= to && matches(record) {
                    result.push(record.clone());
                }
            }
        }

        result
    }
}

/// The time a metric is kept at: the one it started at - or `now`, the time it came, when
/// it names none or one a day ahead
pub fn time_to_keep(started_at: i64, now: i64) -> i64 {
    if started_at <= 0 || started_at > now.saturating_add(MAX_TIME_AHEAD_MICROS) {
        now
    } else {
        started_at
    }
}

/// A record with a wrong time is put at the time it came: it keeps how long its event took.
fn put_in_time(record: &mut Rec, now: i64) {
    let started_at = time_to_keep(record.started_at, now);

    if started_at != record.started_at {
        let duration = record.finished_at.saturating_sub(record.started_at).max(0);
        record.started_at = started_at;
        record.finished_at = started_at.saturating_add(duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics_storage::tests::metric;

    const WINDOW: Duration = Duration::from_secs(5);

    fn scope(name: &str) -> Arc<str> {
        name.into()
    }

    #[test]
    fn a_second_waits_in_memory_before_it_is_ready() {
        let now = Instant::now();
        let now_micros = 1_700_000_000_000_000;
        let mut pending = PendingMetrics::default();

        pending.add(&scope("default"), [metric(now_micros, "billing", "one", 1)], now, now_micros);
        // The same second, a little later: it waits from the first record of the second
        pending.add(
            &scope("default"),
            [metric(now_micros + 5, "billing", "two", 2)],
            now + Duration::from_secs(4),
            now_micros,
        );

        assert!(pending
            .take_ready(now + Duration::from_secs(1), WINDOW, false, |_| {})
            .is_empty());
        assert_eq!(pending.len(), 2);

        // What is done to the records as they are taken is what is written - and found
        let ready = pending.take_ready(now + WINDOW, WINDOW, false, |ready| {
            for record in ready.iter_mut().flat_map(|bucket| bucket.records.iter_mut()) {
                record.event_data.push('!');
            }
        });
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].second, now_micros / MICROS_PER_SECOND);

        let actions = |records: &[Rec]| -> Vec<String> {
            records.iter().map(|record| record.event_data.clone()).collect()
        };
        assert_eq!(actions(&ready[0].records), ["one!", "two!"]);

        // Taken, and not written yet: still counted, still found - and not taken again
        assert_eq!(pending.len(), 2);
        assert_eq!(pending.scopes().count(), 1);
        assert_eq!(
            actions(&pending.search(Some("default"), 0, i64::MAX, |_| true)),
            ["one!", "two!"]
        );
        assert!(pending.take_ready(now + WINDOW, WINDOW, true, |_| {}).is_empty());

        pending.written(&ready);

        assert_eq!(pending.len(), 0);
        assert_eq!(pending.scopes().count(), 0);
        assert!(pending.search(None, 0, i64::MAX, |_| true).is_empty());
    }

    #[test]
    fn the_records_of_a_scope_wait_apart_from_the_others() {
        let now = Instant::now();
        let now_micros = 1_700_000_000_000_000;
        let mut pending = PendingMetrics::default();

        pending.add(&scope("default"), [metric(now_micros, "billing", "one", 1)], now, now_micros);
        pending.add(&scope("trading"), [metric(now_micros, "billing", "one", 1)], now, now_micros);

        assert_eq!(pending.search(Some("trading"), 0, i64::MAX, |_| true).len(), 1);
        assert_eq!(pending.search(Some("prop"), 0, i64::MAX, |_| true).len(), 0);
        assert_eq!(pending.search(None, 0, i64::MAX, |_| true).len(), 2);
        assert_eq!(
            pending.search(None, 0, i64::MAX, |record| record.process_id == 7).len(),
            0
        );

        let ready = pending.take_ready(now, WINDOW, true, |_| {});
        let scopes: Vec<&str> = ready.iter().map(|bucket| &*bucket.scope).collect();
        assert_eq!(scopes, ["default", "trading"]);
    }

    #[test]
    fn a_record_with_a_wrong_time_is_put_at_the_time_it_came() {
        let now_micros = 1_700_000_000_000_000;
        let mut pending = PendingMetrics::default();

        let mut no_time = metric(0, "billing", "one", 1);
        no_time.finished_at = 700;

        let far_ahead = metric(now_micros + 2 * MAX_TIME_AHEAD_MICROS, "billing", "two", 2);

        pending.add(&scope("default"), [no_time, far_ahead], Instant::now(), now_micros);

        let ready = pending.take_ready(Instant::now(), WINDOW, true, |_| {});
        assert_eq!(ready.len(), 1);

        let records = &ready[0].records;
        assert!(records.iter().all(|record| record.started_at == now_micros));
        // How long the event took is kept
        assert_eq!(records[0].finished_at - records[0].started_at, 700);

        // A time which is whatever its writer gave
        for started_at in [i64::MIN, -1, 0, i64::MAX] {
            assert_eq!(time_to_keep(started_at, now_micros), now_micros);
        }
        assert_eq!(time_to_keep(1, now_micros), 1);
        assert_eq!(
            time_to_keep(now_micros + MAX_TIME_AHEAD_MICROS, now_micros),
            now_micros + MAX_TIME_AHEAD_MICROS
        );
    }

    #[test]
    fn the_oldest_seconds_are_dropped_when_the_writes_fall_behind() {
        let now = Instant::now();
        let now_micros = 1_700_000_000_000_000;
        let mut pending = PendingMetrics {
            max_records: 1000,
            ..Default::default()
        };

        let first = (0..10).map(|no| metric(now_micros + no, "billing", "old", no));
        assert_eq!(pending.add(&scope("default"), first, now, now_micros), 0);

        let second = now_micros + MICROS_PER_SECOND;
        let flood = (0..1000).map(|no| metric(second, "billing", "new", no));
        let dropped = pending.add(
            &scope("default"),
            flood,
            now + Duration::from_secs(1),
            now_micros,
        );

        // The second which came first goes, whole
        assert_eq!(dropped, 10);
        assert_eq!(pending.len(), 1000);
        assert!(pending
            .search(None, 0, i64::MAX, |record| record.event_data == "old")
            .is_empty());
    }
}
