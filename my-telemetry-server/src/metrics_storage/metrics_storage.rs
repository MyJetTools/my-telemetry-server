use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use ahash::AHashMap;
use parking_lot::Mutex;
use rust_extensions::date_time::DateTimeAsMicroseconds;
use tokio::sync::{OwnedRwLockReadGuard, RwLock};

use crate::product_scope::{is_valid_scope_name, DEFAULT_PRODUCT_SCOPE};

use super::{
    compress_records, data_file_name, expired_up_to, hour_key_of, hour_start_of,
    hour_start_of_key, is_to_delete_locally, is_to_upload, local_time_is_over, next_wanted,
    second_of, time_of, CompressedBlock, DataFiles, FilterCounts, HourCopies, HourFile,
    HourKeyIndex, HourProcessIndex, HourSource, MetricsQuery, PendingMetrics, ReadyBucket, Rec,
    S3Archive, HOUR_MICROS, KEY_INDEX_FILE_NAME, MICROS_PER_SECOND, PROCESS_INDEX_FILE_NAME,
    SECONDS_PER_HOUR, SECOND_INDEX_FILE_NAME,
};

/// The folder of the metrics under the db path: a folder per product scope in it, a folder
/// per hour in each
pub const METRICS_DIR: &str = "metrics";

/// An hour not read or written for this long is let go - except the current hour and the
/// one before it, whose indexes stay in memory
const IDLE_HOUR_MICROS: i64 = 10 * 60 * MICROS_PER_SECOND;

/// How many events a "last events" screen asks for
pub const LAST_EVENTS_LIMIT: usize = 100;

/// A trace is given this many events at most: what keeps a process id written by mistake
/// into every event from being answered with an hour of them
const PROCESS_EVENTS_LIMIT: usize = 10_000;

/// A trace reads this many seconds of an hour under one lock of it, then lets the writes
/// into the hour through before it reads on
const PROCESS_SCAN_SECONDS: usize = 120;

/// For a trace of an hour whose process index says nothing, which is scanned. A process id
/// is the time its process started, by the clock of the service which started it. The
/// events of the other services of it are stamped by their clocks: one this much behind is
/// still found.
const PROCESS_START_MARGIN: i64 = 60 * MICROS_PER_SECOND;

/// An upload which failed is tried again after this long, and twice as long after each
/// failure more - half an hour at most: a bucket which is down is not asked every minute
/// for every hour which waits for it
const UPLOAD_RETRY: Duration = Duration::from_secs(60);
const MAX_UPLOAD_RETRY: Duration = Duration::from_secs(30 * 60);

/// What the copies of an hour made to be uploaded are named with, next to its files
const UPLOAD_COPY_SUFFIX: &str = ".upload";

/// An hour of a product scope: the scope and where the hour starts
type HourKey = (Arc<str>, i64);

/// An upload of an hour which failed
struct FailedUpload {
    times: u32,
    retry_at: Instant,
}

/// How long after the upload of an hour failed `times` in a row it is tried again
fn upload_retry_delay(times: u32) -> Duration {
    UPLOAD_RETRY
        .saturating_mul(1u32 << times.saturating_sub(1).min(16))
        .min(MAX_UPLOAD_RETRY)
}

enum HourSlot {
    /// Not opened yet, or let go after a write which failed: opened from its files by
    /// whoever needs it next
    Closed,
    Open(HourFile),
    /// Its folder is being deleted, or is: whoever still holds the hour finds nothing in it
    Deleted,
}

struct HourHandle {
    slot: Arc<RwLock<HourSlot>>,
    last_access: AtomicI64,
}

impl HourHandle {
    fn new(now: i64) -> Self {
        Self {
            slot: Arc::new(RwLock::new(HourSlot::Closed)),
            last_access: AtomicI64::new(now),
        }
    }
}

/// An hour as a pass of a search reads it
struct OpenedHour {
    source: HourSource,
    /// Of an hour on the local disk: the writes into it wait while it is held
    _guard: Option<OwnedRwLockReadGuard<HourSlot>>,
}

/// The metrics: by product scope and hour, `{DbPath}/metrics/{scope}/{hour}`. What came in
/// the last seconds waits in memory before it is written. Two hours after an hour ends it
/// is uploaded to S3 when it is set up, to be read from there, and deleted from the local
/// disk either way.
pub struct MetricsStorage {
    root: PathBuf,
    hours: Mutex<BTreeMap<HourKey, Arc<HourHandle>>>,
    pending: Mutex<PendingMetrics>,
    s3: Option<Arc<S3Archive>>,
    failed_uploads: Mutex<BTreeMap<HourKey, FailedUpload>>,
}

impl MetricsStorage {
    pub fn new(db_path: impl AsRef<Path>, s3: Option<S3Archive>) -> Self {
        let root = db_path.as_ref().join(METRICS_DIR);

        if let Err(err) = std::fs::create_dir_all(&root) {
            println!("Can not create {}: {}", root.display(), err);
        }

        Self {
            root,
            hours: Mutex::new(BTreeMap::new()),
            pending: Mutex::new(PendingMetrics::default()),
            s3: s3.map(Arc::new),
            failed_uploads: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn has_s3(&self) -> bool {
        self.s3.is_some()
    }

    fn hour_dir(&self, scope: &str, hour_start: i64) -> PathBuf {
        self.root
            .join(scope)
            .join(hour_key_of(hour_start).to_string())
    }

    /// Takes the metrics a writer sent into memory: they are written once they have waited
    /// there, see [`Self::take_ready`].
    pub fn push(&self, scope: &Arc<str>, records: impl IntoIterator<Item = Rec>) {
        let dropped = self.pending.lock().add(
            scope,
            records,
            Instant::now(),
            DateTimeAsMicroseconds::now().unix_microseconds,
        );

        if dropped > 0 {
            println!(
                "The buffer of metrics to write is full: {} oldest metrics are dropped",
                dropped
            );
        }
    }

    /// Metrics accepted and not written yet: what a crash would cost
    pub fn window_len(&self) -> usize {
        self.pending.lock().len()
    }

    /// Takes the seconds which have waited `window` in memory - all of them with no
    /// window - to be written by [`Self::write_ready`]. `prepare` is given them to do what
    /// is done to the metrics at the last moment before they are written. A search finds
    /// them in memory until they are written.
    pub fn take_ready(
        &self,
        window: Option<Duration>,
        prepare: impl FnOnce(&mut [ReadyBucket]),
    ) -> Vec<Arc<ReadyBucket>> {
        self.pending.lock().take_ready(
            Instant::now(),
            window.unwrap_or(Duration::ZERO),
            window.is_none(),
            prepare,
        )
    }

    /// Writes the seconds taken by [`Self::take_ready`] - and the indexes whatever their
    /// time, when the process is `stopping`
    pub async fn write_ready(self: &Arc<Self>, ready: Vec<Arc<ReadyBucket>>, stopping: bool) {
        if ready.is_empty() && !stopping {
            return;
        }

        let storage = self.clone();

        // The seconds move into the task before anything is awaited: a tick which is
        // given up on while it waits does not take them with it
        let messages = tokio::task::spawn_blocking(move || {
            let mut messages = storage.write(ready);

            if stopping {
                messages.extend(storage.persist_indexes(true));
            }

            messages
        })
        .await
        .unwrap();

        for message in messages {
            println!("{}", message);
        }
    }

    /// Writes everything which waits in memory, and the indexes whatever their time
    #[cfg(test)]
    pub async fn flush_all(self: &Arc<Self>) {
        let ready = self.take_ready(None, |_| {});
        self.write_ready(ready, true).await;
    }

    /// Writes the seconds: each hour under its write lock, and flushed before it is let
    /// go. Returns what went wrong. Blocks: not to be called on a worker of the runtime.
    pub fn write(&self, buckets: Vec<Arc<ReadyBucket>>) -> Vec<String> {
        // The seconds stay in memory for the searches until they are in the files - or
        // are dropped, or the write gives up: then they are let go of, whatever happens
        let _being_written = BeingWritten {
            pending: &self.pending,
            buckets: &buckets,
        };

        let now = DateTimeAsMicroseconds::now().unix_microseconds;
        let mut messages = Vec::new();

        // Of each hour: the second, its records in time order and the records compressed
        let mut by_hour: BTreeMap<HourKey, Vec<(usize, &[Rec], Vec<CompressedBlock>)>> =
            BTreeMap::new();
        let mut late: BTreeMap<HourKey, usize> = BTreeMap::new();

        for bucket in &buckets {
            let time = bucket.second.saturating_mul(MICROS_PER_SECOND);
            let hour_start = hour_start_of(time);
            let key = (bucket.scope.clone(), hour_start);

            if local_time_is_over(hour_start, now) {
                *late.entry(key).or_default() += bucket.records.len();
                continue;
            }

            let Some(second) = second_of(hour_start, time) else {
                continue;
            };

            let records = bucket.records.as_slice();

            // Here, before the lock of the hour is taken: the searches of the hour do not
            // wait for a second to be compressed
            let blocks = compress_records(records);

            by_hour.entry(key).or_default().push((second, records, blocks));
        }

        for ((scope, hour_start), records) in late {
            messages.push(format!(
                "{} metrics of {} are dropped: the hour is not written into any more",
                records,
                hour_name(&scope, hour_start)
            ));
        }

        for ((scope, hour_start), seconds) in by_hour {
            let name = hour_name(&scope, hour_start);
            let records: usize = seconds.iter().map(|(_, records, _)| records.len()).sum();

            let Some(handle) = self.get_hour(&scope, hour_start, true, now) else {
                continue;
            };

            let mut slot = handle.slot.blocking_write();

            let file = match self.open_slot(&mut slot, &scope, hour_start, true) {
                Ok(Some(file)) => file,
                Ok(None) => {
                    messages.push(format!(
                        "{} metrics of {} are dropped: the hour is deleted",
                        records, name
                    ));
                    continue;
                }
                Err(err) => {
                    messages.push(format!(
                        "{} metrics of {} are dropped: the hour does not open: {}",
                        records, name, err
                    ));
                    continue;
                }
            };

            if file.is_sealed() {
                messages.push(format!(
                    "{} metrics of {} are dropped: the hour is being uploaded",
                    records, name
                ));
                continue;
            }

            let mut failed = None;

            for (second, records, blocks) in seconds {
                if let Err(err) = file.write_second(second, records, Some(blocks)) {
                    failed = Some((second, err));
                    break;
                }
            }

            match failed {
                // The hour is half changed in memory. Nothing of it is flushed: it is let
                // go as it is, and opened again from its files - which are as they were.
                Some((second, err)) => {
                    *slot = HourSlot::Closed;
                    messages.push(format!(
                        "Second {} of {} is not written: {}. The {} metrics of the hour being written are lost",
                        second, name, err, records
                    ));
                }
                None => {
                    if let Err(err) = file.flush(false) {
                        messages.push(format!("{} is not flushed: {}", name, err));
                    }
                }
            }
        }

        messages
    }

    /// The hour. Without `create` an hour with no data file is `None`. It is not opened
    /// here: the lock of the map is not held while files are read, let alone rebuilt.
    fn get_hour(
        &self,
        scope: &Arc<str>,
        hour_start: i64,
        create: bool,
        now: i64,
    ) -> Option<Arc<HourHandle>> {
        let key = (scope.clone(), hour_start);
        let mut hours = self.hours.lock();

        if let Some(handle) = hours.get(&key) {
            handle.last_access.store(now, Ordering::Relaxed);
            return Some(handle.clone());
        }

        if !create
            && !self
                .hour_dir(scope, hour_start)
                .join(data_file_name(0))
                .exists()
        {
            return None;
        }

        let handle = Arc::new(HourHandle::new(now));
        hours.insert(key, handle.clone());

        Some(handle)
    }

    /// The files of the hour, opened when they are not yet. `None` - the hour is deleted.
    /// The key index and the process index of an hour which is written into -
    /// `full_indexes` - are read whole.
    fn open_slot<'s>(
        &self,
        slot: &'s mut HourSlot,
        scope: &str,
        hour_start: i64,
        full_indexes: bool,
    ) -> std::io::Result<Option<&'s mut HourFile>> {
        if let HourSlot::Closed = slot {
            *slot = HourSlot::Open(HourFile::open(
                &self.hour_dir(scope, hour_start),
                hour_start,
                full_indexes,
            )?);
        }

        Ok(match slot {
            HourSlot::Open(file) => Some(file),
            HourSlot::Closed | HourSlot::Deleted => None,
        })
    }

    /// The hour as one pass of a search reads it: its local files under its read lock, so
    /// the writes into it wait for the pass - and no longer - or the hour in S3 once it is
    /// there. `None` - there is nothing to read. The index which serves the query is asked
    /// which seconds may match - the key index one by action, the process index a trace;
    /// with no query every second may.
    async fn open_for_reading(
        &self,
        scope: &Arc<str>,
        hour_start: i64,
        query: Option<&MetricsQuery<'_>>,
    ) -> Option<OpenedHour> {
        let now = DateTimeAsMicroseconds::now().unix_microseconds;

        let Some(handle) = self.get_hour(scope, hour_start, false, now) else {
            return self.open_in_s3(scope, hour_start, query).await;
        };

        let closed = matches!(*handle.slot.read().await, HourSlot::Closed);

        if closed {
            let mut slot = handle.slot.write().await;

            // The current hour and the one before are written into: their indexes are
            // kept whole
            let full_indexes = hour_start >= hour_start_of(now) - HOUR_MICROS;

            if let Err(err) = self.open_slot(&mut slot, scope, hour_start, full_indexes) {
                println!("{} does not open: {}", hour_name(scope, hour_start), err);
                return None;
            }
        }

        let guard = handle.slot.clone().read_owned().await;

        let file = match &*guard {
            HourSlot::Open(file) => file,
            // Uploaded, and its folder is gone - or is going - since the hour was asked for
            HourSlot::Deleted => {
                drop(guard);
                return self.open_in_s3(scope, hour_start, query).await;
            }
            HourSlot::Closed => return None,
        };

        if !file.has_entries() {
            return None;
        }

        // The indexes are asked here and let go of: a write into the hour copies a key
        // index a search still holds
        let counts = match query {
            Some(MetricsQuery::Action { name, data, .. }) => {
                file.key_index().counts_of(name, data).await
            }
            Some(MetricsQuery::Process { process_id }) => {
                process_counts(Some(file.process_index()), *process_id).await
            }
            None => FilterCounts::unknown(),
        };

        let source = HourSource {
            hour_start,
            data: file.data_files(),
            cells: Arc::new(file.cells().clone()),
            counts,
        };

        Some(OpenedHour {
            source,
            _guard: Some(guard),
        })
    }

    /// The hour in S3, when it is set up and the hour is there
    async fn open_in_s3(
        &self,
        scope: &Arc<str>,
        hour_start: i64,
        query: Option<&MetricsQuery<'_>>,
    ) -> Option<OpenedHour> {
        let s3 = self.s3.as_ref()?;

        let indexes = match S3Archive::get_indexes(s3, scope, hour_start).await {
            Ok(indexes) => indexes?,
            Err(err) => {
                println!("{} in S3: {}", hour_name(scope, hour_start), err);
                return None;
            }
        };

        let counts = match query {
            Some(MetricsQuery::Action { name, data, .. }) => {
                let key_index = HourKeyIndex::Directory {
                    directory: indexes.key_index.clone(),
                    source: S3Archive::key_index_source(s3, scope, hour_start),
                };

                key_index.counts_of(name, data).await
            }
            Some(MetricsQuery::Process { process_id }) => {
                let process_index =
                    indexes
                        .process_index
                        .as_ref()
                        .map(|directory| HourProcessIndex::Directory {
                            directory: directory.clone(),
                            source: S3Archive::process_index_source(s3, scope, hour_start),
                        });

                process_counts(process_index, *process_id).await
            }
            None => FilterCounts::unknown(),
        };

        let source = HourSource {
            hour_start,
            data: DataFiles::S3 {
                archive: s3.clone(),
                scope: scope.clone(),
                hour_start,
            },
            cells: indexes.cells,
            counts,
        };

        Some(OpenedHour {
            source,
            _guard: None,
        })
    }

    /// The last events of an action of a service in the hour of the scope - of one client,
    /// from `from` on, when they are given - the latest first, [`LAST_EVENTS_LIMIT`] at most.
    pub async fn search_action(
        self: &Arc<Self>,
        scope: Arc<str>,
        hour_start: i64,
        name: String,
        data: String,
        client_id: Option<String>,
        from: Option<i64>,
    ) -> Vec<Rec> {
        let storage = self.clone();
        let runtime = tokio::runtime::Handle::current();

        // The blocks are decompressed as they are read: off the workers of the runtime
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let query = MetricsQuery::Action {
                    name: &name,
                    data: &data,
                    client_id: client_id.as_deref(),
                    from: from.unwrap_or(hour_start).max(hour_start),
                };

                storage
                    .action_events(&scope, hour_start, &query, LAST_EVENTS_LIMIT)
                    .await
            })
        })
        .await
        .unwrap()
    }

    /// Every event of the process in the hour, in every product scope - a process goes
    /// through the services of more than one - the oldest first.
    pub async fn search_process(self: &Arc<Self>, hour_start: i64, process_id: i64) -> Vec<Rec> {
        let storage = self.clone();
        let runtime = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move { storage.process_events(hour_start, process_id).await })
        })
        .await
        .unwrap()
    }

    /// The bottom second of a pass is found in the indexes, which are in memory: down from
    /// the end of the hour until the records which may match add up to `limit` - and the
    /// records to one read at least - and the seconds which may match are read up from it.
    /// When fewer records match than the indexes allow - a client id, a service with too
    /// many actions to index - the next bottom second is found the same way for what is
    /// missing, scaled by the share which matched.
    async fn action_events(
        &self,
        scope: &Arc<str>,
        hour_start: i64,
        query: &MetricsQuery<'_>,
        limit: usize,
    ) -> Vec<Rec> {
        let from = match query {
            MetricsQuery::Action { from, .. } => *from,
            MetricsQuery::Process { .. } => hour_start,
        };
        let to = hour_start + HOUR_MICROS - 1;

        // What waits in memory is looked at before the files are read, not after: a
        // second which is written meanwhile is then found twice - and told apart below -
        // rather than not at all
        let waiting = self
            .pending
            .lock()
            .search(Some(scope), from, to, |record| query.matches(record));

        let mut result = Vec::new();

        if let Some(first_second) = second_of(hour_start, from) {
            let mut window_end = SECONDS_PER_HOUR - 1;
            let mut wanted = limit as u64;

            loop {
                let Some(opened) = self.open_for_reading(scope, hour_start, Some(query)).await
                else {
                    break;
                };

                let source = &opened.source;
                let records_per_read = source.records_per_read();

                let mut window_start = window_end;
                let mut estimated = source.estimate(window_end);
                let mut counted = source.count(window_end);

                while (estimated < wanted || counted < records_per_read)
                    && window_start > first_second
                {
                    window_start -= 1;
                    estimated += source.estimate(window_start);
                    counted += source.count(window_start);
                }

                let mut window = Vec::new();

                for (run_start, run_end) in source.runs(window_start, window_end, records_per_read)
                {
                    source
                        .collect(run_start, run_end, from, to, query, &mut window)
                        .await;
                }

                drop(opened);

                let matched = window.len() as u64;
                result.extend(window);

                if result.len() >= limit || window_start == first_second {
                    break;
                }

                window_end = window_start - 1;
                wanted = next_wanted((limit - result.len()) as u64, estimated, matched);
            }
        }

        // What waits in memory falls into its place among the records of the files
        add_waiting(&mut result, waiting);

        result.sort_by(|a, b| time_of(b).cmp(&time_of(a)));
        result.truncate(limit);
        result
    }

    /// The process index of each scope names the seconds which have events of the process:
    /// only they are read. Where it says nothing - an hour with too many processes to
    /// index, an index which does not read - the hour is scanned from the second the
    /// process started in.
    async fn process_events(&self, hour_start: i64, process_id: i64) -> Vec<Rec> {
        let query = MetricsQuery::Process { process_id };
        let to = hour_start + HOUR_MICROS - 1;

        // Before the files are read, as in a search by action
        let waiting = self
            .pending
            .lock()
            .search(None, hour_start, to, |record| query.matches(record));

        let mut result = Vec::new();

        'scopes: for scope in self.scopes_of_hour(hour_start).await {
            let Some(opened) = self.open_for_reading(&scope, hour_start, Some(&query)).await
            else {
                continue;
            };

            let source = &opened.source;

            let (from, parts) = if source.counts.is_known() {
                let runs = source.runs(0, SECONDS_PER_HOUR - 1, source.records_per_read());
                (hour_start, in_parts(runs, PROCESS_SCAN_SECONDS))
            } else {
                // The process started in this hour: its events are not before it. One
                // which started in another hour - or an id which is not a time - says
                // nothing of where they are: the hour is read whole.
                let first_second = match second_of(hour_start, process_id) {
                    Some(_) => {
                        second_of(hour_start, (process_id - PROCESS_START_MARGIN).max(hour_start))
                            .unwrap_or(0)
                    }
                    None => 0,
                };

                // Nothing is written past the last second which has records
                let last_second = source.cells.last().map_or(0, |(second, _)| second);

                let parts = if first_second <= last_second {
                    in_parts(vec![(first_second, last_second)], PROCESS_SCAN_SECONDS)
                } else {
                    Vec::new()
                };

                (hour_start + first_second as i64 * MICROS_PER_SECOND, parts)
            };

            // The first part is read under the lock the index was asked under, each of
            // the others under one of its own: the writes into the hour get through
            let mut opened = Some(opened);

            for (part_start, part_end) in parts {
                let hour = match opened.take() {
                    Some(hour) => hour,
                    None => match self.open_for_reading(&scope, hour_start, None).await {
                        Some(hour) => hour,
                        None => break,
                    },
                };

                hour.source
                    .collect(part_start, part_end, from, to, &query, &mut result)
                    .await;

                drop(hour);

                if result.len() >= PROCESS_EVENTS_LIMIT {
                    break 'scopes;
                }
            }
        }

        add_waiting(&mut result, waiting);

        result.sort_by_key(time_of);
        result.truncate(PROCESS_EVENTS_LIMIT);
        result
    }

    /// The product scopes there are metrics of - on the local disk, in S3 or waiting in
    /// memory - the default one first, there or not
    pub async fn scopes(self: &Arc<Self>) -> Vec<String> {
        let storage = self.clone();

        // The folders are listed: off the workers of the runtime
        let mut scopes: BTreeSet<String> = tokio::task::spawn_blocking(move || {
            storage
                .local_scopes()
                .iter()
                .map(|scope| scope.to_string())
                .collect()
        })
        .await
        .unwrap();

        scopes.extend(self.pending.lock().scopes().map(|scope| scope.to_string()));

        if let Some(listing) = self.s3_listing().await {
            scopes.extend(listing.scopes().into_iter().map(str::to_string));
        }

        scopes.remove(DEFAULT_PRODUCT_SCOPE);

        std::iter::once(DEFAULT_PRODUCT_SCOPE.to_string())
            .chain(scopes)
            .collect()
    }

    /// The hours of the scope, on the local disk and in S3: where each starts and how
    /// much it takes
    pub async fn hours(self: &Arc<Self>, scope: &str) -> Vec<(i64, u64)> {
        if !is_valid_scope_name(scope) {
            return Vec::new();
        }

        let mut hours: BTreeMap<i64, u64> = BTreeMap::new();

        if let Some(listing) = self.s3_listing().await {
            hours.extend(listing.hours_of(scope));
        }

        let storage = self.clone();
        let local_scope = scope.to_string();

        // The folders of the hours are listed and measured: off the workers of the runtime
        let local = tokio::task::spawn_blocking(move || {
            storage
                .local_hours(&local_scope)
                .into_iter()
                .map(|hour_start| {
                    let size = folder_size(&storage.hour_dir(&local_scope, hour_start));
                    (hour_start, size)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();

        // An hour which is in both is uploaded and not deleted yet: what is read is the
        // local one
        hours.extend(local);

        hours.into_iter().collect()
    }

    /// What S3 has, when it is set up and it answers
    async fn s3_listing(&self) -> Option<Arc<super::ArchiveListing>> {
        self.s3.as_ref()?.listing().await
    }

    /// The product scopes which have a folder
    fn local_scopes(&self) -> Vec<Arc<str>> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };

        entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| is_valid_scope_name(name))
            .map(Arc::from)
            .collect()
    }

    /// The hours the scope has folders of, by where they start
    fn local_hours(&self, scope: &str) -> BTreeSet<i64> {
        let mut result = BTreeSet::new();

        let Ok(entries) = std::fs::read_dir(self.root.join(scope)) else {
            return result;
        };

        for entry in entries.flatten() {
            let hour_start = entry
                .file_name()
                .to_str()
                .filter(|name| name.len() == 10)
                .and_then(|name| name.parse::<i64>().ok())
                .and_then(hour_start_of_key);

            if let Some(hour_start) = hour_start {
                if entry.path().is_dir() {
                    result.insert(hour_start);
                }
            }
        }

        result
    }

    /// The scopes which have the hour, on the local disk or in S3
    async fn scopes_of_hour(&self, hour_start: i64) -> Vec<Arc<str>> {
        let mut scopes: BTreeSet<Arc<str>> = self
            .local_scopes()
            .into_iter()
            .filter(|scope| {
                self.hour_dir(scope, hour_start)
                    .join(data_file_name(0))
                    .exists()
            })
            .collect();

        if let Some(listing) = self.s3_listing().await {
            scopes.extend(listing.scopes_of_hour(hour_start).map(Arc::from));
        }

        scopes.into_iter().collect()
    }

    /// Deletes the local hours whose time is over - with S3 the ones which were not
    /// uploaded by the time they expire - writes the indexes which are due and lets go of
    /// the idle hours. Returns what was done. Blocks: not to be called on a worker of the
    /// runtime.
    pub fn gc(&self, now: DateTimeAsMicroseconds, hours_to_gc: u64) -> Vec<String> {
        let now = now.unix_microseconds;
        let mut messages = Vec::new();

        for scope in self.local_scopes() {
            for hour_start in self.local_hours(&scope) {
                if !is_to_delete_locally(hour_start, now, hours_to_gc, self.s3.is_some()) {
                    continue;
                }

                let name = hour_name(&scope, hour_start);

                match self.delete_local(&scope, hour_start, now) {
                    Ok(()) => messages.push(format!("GC: {} is deleted", name)),
                    Err(err) => messages.push(format!("GC: {} is not deleted: {}", name, err)),
                }
            }
        }

        messages.extend(self.persist_indexes(false));
        self.let_go_idle_hours(now);

        messages
    }

    /// Writes the key indexes and the process indexes of the open hours which are changed -
    /// when it is time for each, or all of them. Returns what went wrong.
    pub fn persist_indexes(&self, force: bool) -> Vec<String> {
        let handles: Vec<(HourKey, Arc<HourHandle>)> = self
            .hours
            .lock()
            .iter()
            .map(|(key, handle)| (key.clone(), handle.clone()))
            .collect();

        let mut messages = Vec::new();

        for ((scope, hour_start), handle) in handles {
            if let HourSlot::Open(file) = &mut *handle.slot.blocking_write() {
                if let Err(err) = file.flush(force) {
                    messages.push(format!(
                        "{} is not flushed: {}",
                        hour_name(&scope, hour_start),
                        err
                    ));
                }
            }
        }

        messages
    }

    /// The folder of the hour goes whole. Whoever holds the hour, or comes for it while it
    /// is being deleted, finds it deleted: it stays in the map until its folder is gone.
    fn delete_local(&self, scope: &Arc<str>, hour_start: i64, now: i64) -> std::io::Result<()> {
        let key = (scope.clone(), hour_start);

        let handle = self
            .hours
            .lock()
            .entry(key.clone())
            .or_insert_with(|| Arc::new(HourHandle::new(now)))
            .clone();

        let result = {
            // A read or a write which holds the hour ends first; the files are let go of
            let mut slot = handle.slot.blocking_write();
            *slot = HourSlot::Deleted;

            let deleted = match std::fs::remove_dir_all(self.hour_dir(scope, hour_start)) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                result => result,
            };

            // The folder of the scope goes with its last hour - it is not deleted while
            // it has one: a scope with no metrics is not one to list
            let _ = std::fs::remove_dir(self.root.join(&**scope));

            deleted
        };

        self.hours.lock().remove(&key);

        result
    }

    /// The hours nobody has used for a while - and nobody holds - are closed. The current
    /// hour and the one before stay: they are written into.
    fn let_go_idle_hours(&self, now: i64) {
        let previous = hour_start_of(now) - HOUR_MICROS;

        self.hours.lock().retain(|(_, hour_start), handle| {
            *hour_start >= previous
                || now - handle.last_access.load(Ordering::Relaxed) < IDLE_HOUR_MICROS
                || Arc::strong_count(handle) > 1
                // A pass which reads the hour holds its slot, not the handle
                || Arc::strong_count(&handle.slot) > 1
        });
    }

    /// The local hours whose time on the local disk is over and which are still kept: to
    /// be uploaded - but the ones whose upload failed, until it is time to try it again.
    /// With `hours_to_gc` of 3 or less none is: they are deleted instead. Blocks: not to
    /// be called on a worker of the runtime.
    pub fn hours_to_upload(
        &self,
        now: DateTimeAsMicroseconds,
        hours_to_gc: u64,
    ) -> Vec<(Arc<str>, i64)> {
        self.hours_to_upload_at(now, hours_to_gc, Instant::now())
    }

    /// `this_moment` is what the uploads which failed wait by
    fn hours_to_upload_at(
        &self,
        now: DateTimeAsMicroseconds,
        hours_to_gc: u64,
        this_moment: Instant,
    ) -> Vec<(Arc<str>, i64)> {
        if self.s3.is_none() {
            return Vec::new();
        }

        let now = now.unix_microseconds;

        let mut result = Vec::new();

        for scope in self.local_scopes() {
            for hour_start in self.local_hours(&scope) {
                if !is_to_upload(hour_start, now, hours_to_gc) {
                    continue;
                }

                let key = (scope.clone(), hour_start);

                let waits = self
                    .failed_uploads
                    .lock()
                    .get(&key)
                    .map_or(false, |failed| failed.retry_at > this_moment);

                if !waits {
                    result.push(key);
                }
            }
        }

        result
    }

    /// Seals the hour - nothing is written into it any more - copies its chain into files
    /// whose frames follow one another, uploads them and deletes the folder of the hour.
    /// An hour which is in S3 already - an upload after which the folder was not deleted -
    /// is not uploaded again. A failed upload unseals the hour, and it is tried again
    /// later - each time later. Returns what was done.
    pub async fn upload_hour(
        self: &Arc<Self>,
        scope: Arc<str>,
        hour_start: i64,
    ) -> Result<String, String> {
        let key = (scope.clone(), hour_start);

        let result = self.upload_and_delete(&scope, hour_start).await;

        let mut failed_uploads = self.failed_uploads.lock();

        match &result {
            Ok(_) => {
                failed_uploads.remove(&key);
            }
            Err(_) => {
                let times = failed_uploads.get(&key).map_or(0, |failed| failed.times) + 1;

                failed_uploads.insert(
                    key,
                    FailedUpload {
                        times,
                        retry_at: Instant::now() + upload_retry_delay(times),
                    },
                );
            }
        }

        result
    }

    async fn upload_and_delete(
        self: &Arc<Self>,
        scope: &Arc<str>,
        hour_start: i64,
    ) -> Result<String, String> {
        let Some(s3) = self.s3.clone() else {
            return Err("S3 is not set up".to_string());
        };

        let name = hour_name(scope, hour_start);

        let already_there = s3.has_hour(scope, hour_start).await?;

        let copies = if already_there {
            None
        } else {
            let storage = self.clone();
            let copied_scope = scope.clone();

            tokio::task::spawn_blocking(move || storage.seal_and_copy(&copied_scope, hour_start))
                .await
                .map_err(|err| err.to_string())??
        };

        let uploaded = match &copies {
            Some(copies) => s3.upload(scope, hour_start, copies).await,
            None => Ok(()),
        };

        let storage = self.clone();
        let scope = scope.clone();

        tokio::task::spawn_blocking(move || {
            let now = DateTimeAsMicroseconds::now().unix_microseconds;

            match uploaded {
                // The copies go with the folder
                Ok(()) => storage
                    .delete_local(&scope, hour_start, now)
                    .map(|_| match (already_there, &copies) {
                        (true, _) => format!("S3: {} is there already: its local files are deleted", name),
                        (false, Some(_)) => format!("S3: {} is uploaded", name),
                        (false, None) => format!("S3: {} has no metrics: it is deleted", name),
                    })
                    .map_err(|err| format!("Uploaded, and not deleted locally: {}", err)),
                Err(err) => {
                    for copy in copies.iter().flat_map(|copies| copies.files()) {
                        let _ = std::fs::remove_file(copy);
                    }

                    storage.set_sealed(&scope, hour_start, false);
                    Err(err)
                }
            }
        })
        .await
        .map_err(|err| err.to_string())?
    }

    /// Returns the copies to upload, next to the files of the hour; `None` - the hour has
    /// no metrics
    fn seal_and_copy(&self, scope: &Arc<str>, hour_start: i64) -> Result<Option<HourCopies>, String> {
        let now = DateTimeAsMicroseconds::now().unix_microseconds;
        let name = hour_name(scope, hour_start);

        // A folder with no data file - what a delete which did not complete leaves - has
        // nothing to upload
        let Some(handle) = self.get_hour(scope, hour_start, false, now) else {
            return Ok(None);
        };

        let mut slot = handle.slot.blocking_write();

        let file = self
            .open_slot(&mut slot, scope, hour_start, false)
            .map_err(|err| err.to_string())?
            .ok_or_else(|| format!("{} is deleted", name))?;

        file.set_sealed(true);
        file.flush(true).map_err(|err| err.to_string())?;

        if !file.has_entries() {
            return Ok(None);
        }

        let dir = self.hour_dir(scope, hour_start);
        let copy = |file: &str| dir.join(format!("{}{}", file, UPLOAD_COPY_SUFFIX));
        let data_copy = |file: u8| copy(&data_file_name(file));

        let mut copies = HourCopies {
            data: Vec::new(),
            second_index: copy(SECOND_INDEX_FILE_NAME),
            key_index: copy(KEY_INDEX_FILE_NAME),
            process_index: copy(PROCESS_INDEX_FILE_NAME),
        };

        let compacted = file.compact_to(
            data_copy,
            &copies.second_index,
            &copies.key_index,
            &copies.process_index,
        );

        match compacted {
            Ok(files) => {
                copies.data = (0..files).map(|file| data_copy(file as u8)).collect();
                Ok(Some(copies))
            }
            // The hour is as it was: not sealed, and with no part of a copy next to it
            Err(err) => {
                file.set_sealed(false);
                remove_upload_copies(&dir);
                Err(err.to_string())
            }
        }
    }

    fn set_sealed(&self, scope: &Arc<str>, hour_start: i64, sealed: bool) {
        let handle = self.hours.lock().get(&(scope.clone(), hour_start)).cloned();

        if let Some(handle) = handle {
            if let HourSlot::Open(file) = &mut *handle.slot.blocking_write() {
                file.set_sealed(sealed);
            }
        }
    }

    /// Deletes the hours of S3 which are not kept any more, of every product scope.
    /// Returns their names.
    pub async fn gc_s3(
        &self,
        now: DateTimeAsMicroseconds,
        hours_to_gc: u64,
    ) -> Result<Vec<String>, String> {
        let Some(s3) = &self.s3 else {
            return Ok(Vec::new());
        };

        let deleted = s3
            .delete_up_to(expired_up_to(now.unix_microseconds, hours_to_gc))
            .await?;

        Ok(deleted
            .into_iter()
            .map(|(scope, hour_start)| hour_name(&scope, hour_start))
            .collect())
    }

    /// The hours to upload as they are a while later: the uploads which failed have waited
    #[cfg(test)]
    pub fn hours_to_upload_after(
        &self,
        now: DateTimeAsMicroseconds,
        hours_to_gc: u64,
        later: Duration,
    ) -> Vec<(Arc<str>, i64)> {
        self.hours_to_upload_at(now, hours_to_gc, Instant::now() + later)
    }

    /// The events of an action of the hour, `limit` of them at most
    #[cfg(test)]
    pub async fn action_events_for_test(
        self: &Arc<Self>,
        scope: &Arc<str>,
        hour_start: i64,
        name: &str,
        data: &str,
        limit: usize,
    ) -> Vec<Rec> {
        let (storage, scope) = (self.clone(), scope.clone());
        let (name, data) = (name.to_string(), data.to_string());
        let runtime = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let query = MetricsQuery::Action {
                    name: &name,
                    data: &data,
                    client_id: None,
                    from: hour_start,
                };

                storage.action_events(&scope, hour_start, &query, limit).await
            })
        })
        .await
        .unwrap()
    }

    /// How many hours are open
    #[cfg(test)]
    pub fn open_hours(&self) -> usize {
        self.hours.lock().len()
    }
}

/// The seconds a write was given: they are in memory for the searches while it lasts, and
/// let go of as it ends - or gives up
struct BeingWritten<'s> {
    pending: &'s Mutex<PendingMetrics>,
    buckets: &'s [Arc<ReadyBucket>],
}

impl Drop for BeingWritten<'_> {
    fn drop(&mut self) {
        self.pending.lock().written(self.buckets);
    }
}

/// Adds to the records read from the files the ones which waited in memory when the search
/// began - but for the ones written since, which the files gave already. A record is told
/// by its process, its times, its service and its action: the tags are not looked at - a
/// record gets its `client_id` as it is written.
fn add_waiting(from_files: &mut Vec<Rec>, waiting: Vec<Rec>) {
    if waiting.is_empty() {
        return;
    }

    fn id_of(record: &Rec) -> (i64, i64, i64, &str, &str) {
        (
            record.process_id,
            record.started_at,
            record.finished_at,
            record.service_name.as_str(),
            record.event_data.as_str(),
        )
    }

    let still_waiting: Vec<Rec> = {
        // How many times the files gave each record. A `Cell`: the map is looked into by
        // the ids of other records, which a mutable borrow of it does not let.
        let mut written: AHashMap<(i64, i64, i64, &str, &str), Cell<usize>> = AHashMap::new();

        for record in from_files.iter() {
            *written.entry(id_of(record)).or_default().get_mut() += 1;
        }

        waiting
            .into_iter()
            .filter(|record| match written.get(&id_of(record)) {
                Some(times) if times.get() > 0 => {
                    times.set(times.get() - 1);
                    false
                }
                _ => true,
            })
            .collect()
    };

    from_files.extend(still_waiting);
}

/// The copies of an upload which did not complete
fn remove_upload_copies(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .ends_with(UPLOAD_COPY_SUFFIX)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// What the process index of an hour says of a trace: the seconds to read - or nothing,
/// when the hour has none, is not indexed by process, or its index does not read
async fn process_counts(index: Option<HourProcessIndex<'_>>, process_id: i64) -> FilterCounts {
    let seconds = match index {
        Some(index) => index.seconds_of(process_id).await,
        None => None,
    };

    match seconds {
        Some(seconds) => FilterCounts::of_seconds(&seconds),
        None => FilterCounts::unknown(),
    }
}

/// The runs of seconds as parts of `max_seconds` at most
fn in_parts(runs: Vec<(usize, usize)>, max_seconds: usize) -> Vec<(usize, usize)> {
    let mut result = Vec::with_capacity(runs.len());

    for (run_start, run_end) in runs {
        let mut part_start = run_start;

        while part_start <= run_end {
            let part_end = (part_start + max_seconds - 1).min(run_end);
            result.push((part_start, part_end));
            part_start = part_end + 1;
        }
    }

    result
}

/// `trading/2026101112`
fn hour_name(scope: &str, hour_start: i64) -> String {
    format!("{}/{}", scope, hour_key_of(hour_start))
}

/// What the files of the hour take - the copies an upload makes and leaves aside
fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.ends_with(UPLOAD_COPY_SUFFIX) && !name.ends_with(".tmp")
        })
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::in_parts;

    #[test]
    fn a_failed_upload_is_tried_later_each_time() {
        use super::upload_retry_delay;
        use std::time::Duration;

        let minutes: Vec<u64> = (1..=8)
            .map(|times| upload_retry_delay(times).as_secs() / 60)
            .collect();

        assert_eq!(minutes, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(upload_retry_delay(u32::MAX), Duration::from_secs(30 * 60));
    }

    #[test]
    fn long_runs_are_read_in_parts() {
        assert_eq!(in_parts(vec![(5, 5), (10, 12)], 120), [(5, 5), (10, 12)]);
        assert_eq!(
            in_parts(vec![(0, 250), (300, 300)], 120),
            [(0, 119), (120, 239), (240, 250), (300, 300)]
        );
        assert!(in_parts(Vec::new(), 120).is_empty());
    }
}
