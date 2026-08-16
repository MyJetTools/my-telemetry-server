use rust_extensions::date_time::{HourKey, IntervalKey};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::db::MetricDto;

use super::*;

/// Read granularity for a scan. An hour file is expected to weigh up to ~1Gb, so it is
/// never pulled into memory whole.
const SCAN_CHUNK_LEN: usize = 1024 * 1024;

/// The records themselves, in `[u16 len][payload]` form.
pub const DATA_FILE_NAME: &str = "metrics.data";

/// The 3600 slot index over [`DATA_FILE_NAME`]. Named by the axis it indexes, so a second
/// index on another axis reads consistently next to it.
pub const SECOND_INDEX_FILE_NAME: &str = "second.index";

/// Folder holding one hour, named by its hour key - `2026010512`.
pub fn hour_folder(db_path: &str, hour_key: IntervalKey<HourKey>) -> String {
    format!(
        "{}{}{}",
        db_path,
        std::path::MAIN_SEPARATOR,
        hour_key.to_i64()
    )
}

pub fn data_file_name(db_path: &str, hour_key: IntervalKey<HourKey>) -> String {
    format!(
        "{}{}{}",
        hour_folder(db_path, hour_key),
        std::path::MAIN_SEPARATOR,
        DATA_FILE_NAME
    )
}

/// Everything for one hour: the data file, its second index, and the tail of events that
/// has not reached disk yet.
///
/// All logic lives here and is plain `&mut self` - the borrow checker keeps the index and
/// the data file consistent with each other. The wrapper owns the lock and nothing else.
pub(super) struct StorageByHourInner {
    hour_key: IntervalKey<HourKey>,
    folder: String,
    data_file_name: String,
    index_file_name: String,
    data_file: Option<File>,
    index_file: Option<File>,
    index: SecondIndex,
    data_len: u64,
    /// Latest second that reached disk. This is what makes an incoming record an append
    /// or a heavy insert - not the index, which cannot tell "same second, later record".
    last_written_second: Option<usize>,
    opened: bool,
    /// The last-minute buffer, sorted by `started`. Out of order arrivals are absorbed
    /// here, which is why the data file is append-only by construction. Every read has to
    /// merge it in, or the UI would not see what just happened.
    window: Vec<MetricDto>,
}

impl StorageByHourInner {
    pub(super) fn new(hour_key: IntervalKey<HourKey>, db_path: &str) -> Self {
        let folder = hour_folder(db_path, hour_key);

        Self {
            hour_key,
            data_file_name: format!("{}{}{}", folder, std::path::MAIN_SEPARATOR, DATA_FILE_NAME),
            index_file_name: format!(
                "{}{}{}",
                folder,
                std::path::MAIN_SEPARATOR,
                SECOND_INDEX_FILE_NAME
            ),
            folder,
            data_file: None,
            index_file: None,
            index: SecondIndex::new(),
            data_len: 0,
            last_written_second: None,
            opened: false,
            window: Vec::new(),
        }
    }

    /// The folder that holds this hour. Deleting it takes the data and every index with
    /// it, which is the whole point of an hour being a folder rather than loose files.
    pub(super) fn folder(&self) -> &str {
        &self.folder
    }

    pub(super) fn window_len(&self) -> usize {
        self.window.len()
    }

    /// Makes the hour exist on disk, empty. Cheap after the first call - `open` is a no-op
    /// once the handles are held.
    pub(super) async fn ensure_created(&mut self) {
        self.open(true).await;
    }

    /// Accepts a metric into the in-memory window, keeping it sorted by `started`.
    ///
    /// Almost always an append at the tail, so the backward search settles on the first
    /// comparison; a late arrival costs a `Vec::insert`, which is a memmove of the window
    /// and nothing more.
    pub(super) fn push(&mut self, dto: MetricDto) {
        let at = match self
            .window
            .iter()
            .rposition(|itm| itm.started <= dto.started)
        {
            Some(index) => index + 1,
            None => 0,
        };

        self.window.insert(at, dto);
    }

    /// Moves everything older than `flush_before` out of the window and onto disk.
    pub(super) async fn flush(&mut self, flush_before: i64) {
        let split = self
            .window
            .partition_point(|itm| itm.started < flush_before);

        if split == 0 {
            return;
        }

        let to_write: Vec<MetricDto> = self.window.drain(..split).collect();

        self.write_to_disk(to_write).await;
    }

    /// Moves the whole window to disk regardless of age - used when the hour is closing.
    pub(super) async fn flush_all(&mut self) {
        if self.window.is_empty() {
            return;
        }

        let to_write: Vec<MetricDto> = self.window.drain(..).collect();

        self.write_to_disk(to_write).await;
    }

    /// Scans the hour from `from_second` onward and returns everything the filter keeps.
    ///
    /// The disk is streamed in chunks, then the in-memory window is merged in. The result
    /// is ordered by `started` ascending, because both sources already are.
    ///
    /// `keep_last` bounds what the scan holds. A "last 100 events" screen must not make the
    /// server materialize every match in a ~1Gb hour just to throw all but 100 away; with
    /// it set, older matches are dropped as newer ones are found.
    pub(super) async fn scan<TFilter: Fn(&MetricDto) -> bool>(
        &mut self,
        from_second: usize,
        keep_last: Option<usize>,
        filter: TFilter,
    ) -> Vec<MetricDto> {
        let mut result = Vec::new();

        self.scan_disk(from_second, keep_last, &filter, &mut result)
            .await;

        for itm in self.window.iter() {
            if second_within_hour(itm.started) < from_second {
                continue;
            }

            if filter(itm) {
                result.push(itm.clone());
            }
        }

        // The batched trim leaves up to twice the limit; the caller gets exactly it.
        if let Some(keep_last) = keep_last {
            if result.len() > keep_last {
                let excess = result.len() - keep_last;
                result.drain(..excess);
            }
        }

        result
    }

    /// Closes the files so they can be deleted, and drops everything held for this hour.
    pub(super) fn close(&mut self) {
        self.data_file = None;
        self.index_file = None;
        self.window.clear();
        self.index.reset();
        self.data_len = 0;
        self.last_written_second = None;
        self.opened = false;
    }

    async fn scan_disk<TFilter: Fn(&MetricDto) -> bool>(
        &mut self,
        from_second: usize,
        keep_last: Option<usize>,
        filter: &TFilter,
        out: &mut Vec<MetricDto>,
    ) {
        if !self.open(false).await {
            return;
        }

        let Some(from) = self.index.offset_at_or_after(from_second) else {
            return;
        };

        let file = self.data_file.as_mut().unwrap();

        if let Err(err) = file.seek(std::io::SeekFrom::Start(from)).await {
            println!(
                "Failed to seek {} to {}: {:?}",
                self.data_file_name, from, err
            );
            return;
        }

        let mut pending: Vec<u8> = Vec::with_capacity(SCAN_CHUNK_LEN * 2);
        let mut chunk = vec![0u8; SCAN_CHUNK_LEN];

        loop {
            let read = match file.read(&mut chunk).await {
                Ok(read) => read,
                Err(err) => {
                    println!("Failed to read {}: {:?}", self.data_file_name, err);
                    return;
                }
            };

            if read == 0 {
                break;
            }

            pending.extend_from_slice(&chunk[..read]);

            let mut iterator = RecordsIterator::new(&pending);

            for (_, payload) in iterator.by_ref() {
                if let Some(dto) = decode(payload) {
                    if filter(&dto) {
                        out.push(dto);
                    }
                }
            }

            let consumed = iterator.position();
            pending.drain(..consumed);

            trim_to_last(out, keep_last);
        }
    }

    async fn write_to_disk(&mut self, items: Vec<MetricDto>) {
        if !self.open(true).await {
            return;
        }

        // Index slots are u32. Past that the offsets would silently wrap and the index
        // would point at the middle of records - refusing loudly beats corrupting an hour.
        // An hour is expected to weigh ~1Gb, so reaching this means something else is wrong.
        if self.data_len >= MAX_DATA_FILE_LEN {
            println!(
                "Hour {} reached {} bytes, past what a u32 index slot can address. {} metrics dropped.",
                self.hour_key.to_i64(),
                self.data_len,
                items.len()
            );
            return;
        }

        // Consecutive appends are batched into one write - the normal path never touches
        // the disk more than once per flush.
        let mut append_buffer: Vec<u8> = Vec::new();
        let mut append_starts: Vec<(usize, u64)> = Vec::new();

        for dto in items {
            let second = second_within_hour(dto.started);
            let bytes = encode(dto);

            let is_append = match self.last_written_second {
                Some(last) => second >= last,
                None => true,
            };

            if is_append {
                let offset = self.data_len + append_buffer.len() as u64;
                append_starts.push((second, offset));
                append_buffer.extend_from_slice(&bytes);
                continue;
            }

            // Late beyond the window. Everything buffered so far has to land first, or the
            // offsets the insert is about to shift would be wrong.
            self.commit_appends(&mut append_buffer, &mut append_starts)
                .await;

            self.heavy_insert(second, bytes).await;
        }

        self.commit_appends(&mut append_buffer, &mut append_starts)
            .await;

        self.write_index().await;
    }

    async fn commit_appends(
        &mut self,
        append_buffer: &mut Vec<u8>,
        append_starts: &mut Vec<(usize, u64)>,
    ) {
        if append_buffer.is_empty() {
            return;
        }

        let file = self.data_file.as_mut().unwrap();

        if let Err(err) = file.seek(std::io::SeekFrom::Start(self.data_len)).await {
            println!(
                "Failed to seek {} to append: {:?}",
                self.data_file_name, err
            );
            append_buffer.clear();
            append_starts.clear();
            return;
        }

        if let Err(err) = file.write_all(append_buffer).await {
            println!("Failed to append to {}: {:?}", self.data_file_name, err);
            append_buffer.clear();
            append_starts.clear();
            return;
        }

        let _ = file.sync_data().await;

        self.data_len += append_buffer.len() as u64;

        for (second, offset) in append_starts.iter() {
            self.index.set_start_of_second(*second, *offset);
            self.last_written_second = Some(*second);
        }

        append_buffer.clear();
        append_starts.clear();
    }

    /// The rare path: a record older than everything on disk. The tail from the start of
    /// its second is streamed into memory, the record is placed in it, and the tail goes
    /// back. Every later index slot moves by exactly the record size.
    async fn heavy_insert(&mut self, second: usize, bytes: Vec<u8>) {
        let Some(from) = self.index.offset_at_or_after(second) else {
            // Nothing at or after this second - it is an append after all.
            let offset = self.data_len;
            let file = self.data_file.as_mut().unwrap();
            if file.seek(std::io::SeekFrom::Start(offset)).await.is_ok()
                && file.write_all(&bytes).await.is_ok()
            {
                self.data_len += bytes.len() as u64;
                self.index.set_start_of_second(second, offset);
            }
            return;
        };

        let tail_len = self.data_len - from;

        println!(
            "Late metric for hour {} second {}: rewriting {} bytes of tail",
            self.hour_key.to_i64(),
            second,
            tail_len
        );

        let Some(tail) = self.read_range(from, tail_len as usize).await else {
            return;
        };

        let started = match decode(&bytes[LEN_PREFIX_SIZE..]) {
            Some(dto) => dto.started,
            None => return,
        };

        let at = insert_position(&tail, started);

        let mut rewritten = Vec::with_capacity(tail.len() + bytes.len());
        rewritten.extend_from_slice(&tail[..at]);
        rewritten.extend_from_slice(&bytes);
        rewritten.extend_from_slice(&tail[at..]);

        let file = self.data_file.as_mut().unwrap();

        if let Err(err) = file.seek(std::io::SeekFrom::Start(from)).await {
            println!(
                "Failed to seek {} for insert: {:?}",
                self.data_file_name, err
            );
            return;
        }

        if let Err(err) = file.write_all(&rewritten).await {
            println!(
                "Failed to rewrite tail of {}: {:?}",
                self.data_file_name, err
            );
            return;
        }

        let _ = file.sync_data().await;

        let delta = bytes.len() as u64;

        self.data_len += delta;
        self.index.set_start_of_second(second, from + at as u64);
        self.index.shift_after(second, delta);
    }

    async fn read_range(&mut self, from: u64, len: usize) -> Option<Vec<u8>> {
        let file = self.data_file.as_mut()?;

        if let Err(err) = file.seek(std::io::SeekFrom::Start(from)).await {
            println!("Failed to seek {}: {:?}", self.data_file_name, err);
            return None;
        }

        let mut result = vec![0u8; len];

        if let Err(err) = file.read_exact(&mut result).await {
            println!("Failed to read {}: {:?}", self.data_file_name, err);
            return None;
        }

        Some(result)
    }

    /// The whole index is 14.4Kb - rewriting it wholesale is cheaper than working out
    /// which slots moved, and it cannot go out of step with the data.
    async fn write_index(&mut self) {
        let bytes = self.index.as_bytes();

        let Some(file) = self.index_file.as_mut() else {
            return;
        };

        if let Err(err) = file.seek(std::io::SeekFrom::Start(0)).await {
            println!("Failed to seek {}: {:?}", self.index_file_name, err);
            return;
        }

        if let Err(err) = file.write_all(&bytes).await {
            println!("Failed to write {}: {:?}", self.index_file_name, err);
            return;
        }

        let _ = file.sync_data().await;
    }

    /// Opens both files. With `create` false an hour nobody ever wrote to stays closed and
    /// reads return nothing - that is an empty result, not an error.
    async fn open(&mut self, create: bool) -> bool {
        if self.opened {
            return true;
        }

        if !create && !std::path::Path::new(self.data_file_name.as_str()).exists() {
            return false;
        }

        if create {
            if let Err(err) = tokio::fs::create_dir_all(self.folder.as_str()).await {
                println!("Failed to create folder {}: {:?}", self.folder, err);
                return false;
            }
        }

        let data_file = match open_file(self.data_file_name.as_str(), create).await {
            Some(file) => file,
            None => return false,
        };

        // The index is derived data and it is ours to write, so it is always opened with
        // `create`. A missing index is a rebuild, never a reason to fail the read - and if
        // it cannot be opened at all, the hour still serves from an index held in memory.
        let index_file = open_file(self.index_file_name.as_str(), true).await;

        self.data_len = data_file.metadata().await.map(|itm| itm.len()).unwrap_or(0);

        self.data_file = Some(data_file);
        self.index_file = index_file;
        self.opened = true;

        self.load_index().await;

        true
    }

    async fn load_index(&mut self) {
        let mut bytes = Vec::new();

        let loaded = match self.index_file.as_mut() {
            Some(file) => {
                file.seek(std::io::SeekFrom::Start(0)).await.is_ok()
                    && file.read_to_end(&mut bytes).await.is_ok()
            }
            None => false,
        };

        if loaded {
            if let Some(index) = SecondIndex::from_bytes(&bytes) {
                self.index = index;
                self.last_written_second = self.index.last_non_empty_second();
                return;
            }
        }

        self.rebuild_index().await;
    }

    /// The index is derived data - a missing or torn one is rebuilt by walking the data
    /// file, which is why losing it on a crash is not data loss.
    async fn rebuild_index(&mut self) {
        self.index.reset();
        self.last_written_second = None;

        if self.data_len == 0 {
            self.write_index().await;
            return;
        }

        println!(
            "Rebuilding second index for hour {} from {} bytes of data",
            self.hour_key.to_i64(),
            self.data_len
        );

        let Some(file) = self.data_file.as_mut() else {
            return;
        };

        if file.seek(std::io::SeekFrom::Start(0)).await.is_err() {
            return;
        }

        let mut pending: Vec<u8> = Vec::with_capacity(SCAN_CHUNK_LEN * 2);
        let mut chunk = vec![0u8; SCAN_CHUNK_LEN];
        let mut consumed_total: u64 = 0;

        loop {
            let read = match file.read(&mut chunk).await {
                Ok(read) => read,
                Err(_) => break,
            };

            if read == 0 {
                break;
            }

            pending.extend_from_slice(&chunk[..read]);

            let mut iterator = RecordsIterator::new(&pending);
            let mut found: Vec<(usize, u64)> = Vec::new();

            for (at, payload) in iterator.by_ref() {
                if let Some(dto) = decode(payload) {
                    found.push((second_within_hour(dto.started), consumed_total + at as u64));
                }
            }

            let consumed = iterator.position();

            for (second, offset) in found {
                self.index.set_start_of_second(second, offset);
                self.last_written_second = Some(second);
            }

            consumed_total += consumed as u64;
            pending.drain(..consumed);
        }

        // A torn tail after a crash: everything past the last whole record is unreadable,
        // so the file ends where the records do.
        if consumed_total < self.data_len {
            println!(
                "Dropping {} torn bytes off the tail of {}",
                self.data_len - consumed_total,
                self.data_file_name
            );

            if let Some(file) = self.data_file.as_mut() {
                if file.set_len(consumed_total).await.is_ok() {
                    self.data_len = consumed_total;
                }
            }
        }

        self.write_index().await;
    }
}

/// Drops everything but the newest `keep_last` matches.
///
/// Trimming in batches rather than on every push keeps this amortized: the buffer is
/// allowed to grow to twice the limit before the front is dropped in one `drain`.
fn trim_to_last(out: &mut Vec<MetricDto>, keep_last: Option<usize>) {
    let Some(keep_last) = keep_last else {
        return;
    };

    if out.len() >= keep_last * 2 {
        let excess = out.len() - keep_last;
        out.drain(..excess);
    }
}

/// Byte offset within `tail` where a record with `started` belongs, keeping the tail
/// sorted. Records are walked in order and the first one that is strictly later wins.
fn insert_position(tail: &[u8], started: i64) -> usize {
    for (at, payload) in RecordsIterator::new(tail) {
        if let Some(dto) = decode(payload) {
            if dto.started > started {
                return at;
            }
        }
    }

    tail.len()
}

async fn open_file(file_name: &str, create: bool) -> Option<File> {
    let result = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .open(file_name)
        .await;

    match result {
        Ok(file) => Some(file),
        Err(err) => {
            println!("Failed to open {}: {:?}", file_name, err);
            None
        }
    }
}
