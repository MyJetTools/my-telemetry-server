use std::{collections::BTreeSet, io::Read, path::Path, sync::Arc};

use super::{read_bytes, second_of, DataSource, Rec, READ_LEN, SECONDS_PER_HOUR};

/// A process and a second of the hour which has events of it: the process id big-endian
/// with its sign bit flipped, then the second big-endian - bytes which sort by process id,
/// then by second.
type Pair = [u8; PAIR_LEN];

const PAIR_LEN: usize = 10;

/// `total_records u64 · pairs u64 · format u8 · let_go u8`
const HEADER_LEN: usize = 18;

const FORMAT: u8 = 1;

/// The pairs of the file are read by the page: a trace reads the one its process is in
const PAIRS_PER_PAGE: usize = 4096;

/// With more pairs than this the index of the hour is let go: it is in memory whole for
/// the hours which are written, about 20 bytes a pair
const MAX_PAIRS: usize = 4_000_000;

fn pair_of(process_id: i64, second: u16) -> Pair {
    let mut pair = [0u8; PAIR_LEN];
    pair[..8].copy_from_slice(&((process_id as u64) ^ (1 << 63)).to_be_bytes());
    pair[8..].copy_from_slice(&second.to_be_bytes());
    pair
}

fn second_of_pair(pair: &[u8]) -> usize {
    u16::from_be_bytes([pair[8], pair[9]]) as usize
}

/// The seconds kept for the process and the one its id names, in order
fn with_start_second(hour_start: i64, process_id: i64, mut seconds: Vec<usize>) -> Vec<usize> {
    if let Some(start) = second_of(hour_start, process_id) {
        if let Err(at) = seconds.binary_search(&start) {
            seconds.insert(at, start);
        }
    }

    seconds
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    total_records: u64,
    pairs: usize,
    let_go: bool,
}

impl Header {
    /// `None` while the header is cut, or when it is not of this format
    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN || bytes[16] != FORMAT {
            return None;
        }

        let pairs = u64::from_le_bytes(bytes[8..16].try_into().unwrap());

        // More than an index ever holds: not a number of pairs
        if pairs > MAX_PAIRS as u64 {
            return None;
        }

        Some(Self {
            total_records: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            pairs: pairs as usize,
            let_go: bytes[17] != 0,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.total_records.to_le_bytes());
        out.extend_from_slice(&(self.pairs as u64).to_le_bytes());
        out.push(FORMAT);
        out.push(self.let_go as u8);
    }

    fn pages(&self) -> usize {
        self.pairs.div_ceil(PAIRS_PER_PAGE)
    }

    /// Where the directory ends in the file and the pairs start
    fn directory_end(&self) -> usize {
        HEADER_LEN + self.pages() * PAIR_LEN
    }

    fn file_len(&self) -> usize {
        self.directory_end() + self.pairs * PAIR_LEN
    }
}

/// Which seconds of the hour have events of each process - what a trace is read by.
///
/// A process id is the time its process started, so the second the id itself names is
/// where most of its events are. That second is not kept: a trace always reads it. What is
/// kept are the other seconds - of a process which runs on into the next ones, of a
/// service whose clock is a little off, of an id which is not a time. A process which is
/// over within the second it started in takes nothing here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIndex {
    hour_start: i64,
    pairs: BTreeSet<Pair>,
    /// The records counted: the counts of the index of the hour add up to it when the two
    /// agree
    total_records: u64,
    /// The hour has more pairs than an hour is indexed within: its traces scan it
    let_go: bool,
    max_pairs: usize,
}

impl ProcessIndex {
    pub fn new(hour_start: i64) -> Self {
        Self {
            hour_start,
            pairs: BTreeSet::new(),
            total_records: 0,
            let_go: false,
            max_pairs: MAX_PAIRS,
        }
    }

    pub fn total_records(&self) -> u64 {
        self.total_records
    }

    /// So many of the records counted were not written - they fit no entry: the hour has
    /// that many fewer. The second stays named for their processes: one read for nothing.
    pub fn forget_records(&mut self, records: usize) {
        self.total_records = self.total_records.saturating_sub(records as u64);
    }

    pub fn add_records(&mut self, second: usize, records: &[Rec]) {
        self.total_records += records.len() as u64;

        if self.let_go {
            return;
        }

        for record in records {
            // The second the id names is read by every trace of it
            if second_of(self.hour_start, record.process_id) != Some(second) {
                self.pairs.insert(pair_of(record.process_id, second as u16));
            }
        }

        if self.pairs.len() > self.max_pairs {
            println!("The hour has too many processes to index: its traces scan it");
            self.pairs = BTreeSet::new();
            self.let_go = true;
        }
    }

    /// The seconds which have events of the process, in order. `None` - the hour is not
    /// indexed by process.
    pub fn seconds_of(&self, process_id: i64) -> Option<Vec<usize>> {
        if self.let_go {
            return None;
        }

        let kept = self
            .pairs
            .range(pair_of(process_id, 0)..=pair_of(process_id, u16::MAX))
            .map(|pair| second_of_pair(pair))
            .collect();

        Some(with_start_second(self.hour_start, process_id, kept))
    }

    /// How many pairs are kept
    #[cfg(test)]
    pub fn pairs(&self) -> usize {
        self.pairs.len()
    }

    /// The header, then the directory - the first pair of each page - then the pairs,
    /// sorted, [`PAIRS_PER_PAGE`] to a page.
    pub fn to_file(&self) -> Vec<u8> {
        let header = Header {
            total_records: self.total_records,
            pairs: self.pairs.len(),
            let_go: self.let_go,
        };

        let mut result = Vec::with_capacity(header.file_len());
        header.write(&mut result);

        for pair in self.pairs.iter().step_by(PAIRS_PER_PAGE) {
            result.extend_from_slice(pair);
        }

        for pair in &self.pairs {
            result.extend_from_slice(pair);
        }

        result
    }

    /// `None` when the bytes are not a whole index of this format
    pub fn from_file(hour_start: i64, bytes: &[u8]) -> Option<Self> {
        let header = Header::parse(bytes)?;

        if bytes.len() != header.file_len() {
            return None;
        }

        let pairs_bytes = &bytes[header.directory_end()..];
        let mut before: Option<&[u8]> = None;

        for pair in pairs_bytes.chunks_exact(PAIR_LEN) {
            if before.map_or(false, |before| before >= pair)
                || second_of_pair(pair) >= SECONDS_PER_HOUR
            {
                return None;
            }

            before = Some(pair);
        }

        Some(Self {
            hour_start,
            pairs: pairs_bytes
                .chunks_exact(PAIR_LEN)
                .map(|pair| pair.try_into().unwrap())
                .collect(),
            total_records: header.total_records,
            let_go: header.let_go,
            max_pairs: MAX_PAIRS,
        })
    }
}

/// The process index of an hour as a trace reads it, from a file or from S3 alike: the
/// first pair of each of its pages in memory, the page of a process read when it is asked
/// for.
pub struct ProcessIndexDirectory {
    hour_start: i64,
    header: Header,
    first_pairs: Vec<Pair>,
}

impl ProcessIndexDirectory {
    pub fn total_records(&self) -> u64 {
        self.header.total_records
    }

    /// `bytes` start with the header and hold the directory whole
    fn parse(hour_start: i64, header: Header, bytes: &[u8]) -> Option<Self> {
        let directory = bytes.get(HEADER_LEN..header.directory_end())?;

        Some(Self {
            hour_start,
            header,
            first_pairs: directory
                .chunks_exact(PAIR_LEN)
                .map(|pair| pair.try_into().unwrap())
                .collect(),
        })
    }

    /// The directory of a local file; `None` when it does not read, or the file is not as
    /// long as its header says - a write of it which did not complete
    pub fn read_local(hour_start: i64, path: &Path) -> Option<Self> {
        let mut file = std::fs::File::open(path).ok()?;

        let mut bytes = vec![0u8; HEADER_LEN];
        file.read_exact(&mut bytes).ok()?;

        let header = Header::parse(&bytes)?;

        if file.metadata().ok()?.len() != header.file_len() as u64 {
            return None;
        }

        bytes.resize(header.directory_end(), 0);
        file.read_exact(&mut bytes[HEADER_LEN..]).ok()?;

        Self::parse(hour_start, header, &bytes)
    }

    /// The directory of the file: one read of 64 KiB, and the rest of it when it is longer
    pub async fn read(hour_start: i64, source: &DataSource) -> Result<Self, String> {
        let mut bytes = read_bytes(source, 0, READ_LEN).await?;

        let header =
            Header::parse(&bytes).ok_or_else(|| "The process index is cut".to_string())?;

        if header.directory_end() > bytes.len() {
            let rest = read_bytes(
                source,
                bytes.len() as u64,
                header.directory_end() - bytes.len(),
            )
            .await?;
            bytes.extend_from_slice(&rest);
        }

        Self::parse(hour_start, header, &bytes)
            .ok_or_else(|| "The directory of the process index is cut".to_string())
    }

    /// The seconds which have events of the process, in order: the pages its pairs may be
    /// in are read - one, or two when they go over from a page to the next. `None` - the
    /// hour is not indexed by process.
    async fn seconds_of(
        &self,
        source: &DataSource,
        process_id: i64,
    ) -> Result<Option<Vec<usize>>, String> {
        if self.header.let_go {
            return Ok(None);
        }

        let first = pair_of(process_id, 0);
        let last = pair_of(process_id, u16::MAX);

        // The pages which start at or before the last pair the process may have
        let pages_up_to = self.first_pairs.partition_point(|pair| *pair <= last);

        if pages_up_to == 0 {
            return Ok(Some(with_start_second(self.hour_start, process_id, Vec::new())));
        }

        let from_page = self
            .first_pairs
            .partition_point(|pair| *pair <= first)
            .saturating_sub(1);

        let from_pair = from_page * PAIRS_PER_PAGE;
        let to_pair = (pages_up_to * PAIRS_PER_PAGE).min(self.header.pairs);
        let len = (to_pair - from_pair) * PAIR_LEN;

        let offset = self.header.directory_end() + from_pair * PAIR_LEN;
        let bytes = read_bytes(source, offset as u64, len).await?;

        if bytes.len() < len {
            return Err("The process index ends inside its pairs".to_string());
        }

        let pair_at = |index: usize| &bytes[index * PAIR_LEN..(index + 1) * PAIR_LEN];
        let count = to_pair - from_pair;

        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = (low + high) / 2;

            if pair_at(middle) < &first[..] {
                low = middle + 1;
            } else {
                high = middle;
            }
        }

        let mut kept = Vec::new();

        while low < count && pair_at(low) <= &last[..] {
            let second = second_of_pair(pair_at(low));

            if second >= SECONDS_PER_HOUR {
                return Err("A second of the process index is not one of an hour".to_string());
            }

            kept.push(second);
            low += 1;
        }

        Ok(Some(with_start_second(self.hour_start, process_id, kept)))
    }
}

/// The process index of an hour as a trace uses it
pub enum HourProcessIndex<'s> {
    /// The hour is written into: the whole index, in memory
    Full(&'s ProcessIndex),
    /// The hour is read: the directory in memory, the pages read from `source` when asked
    Directory {
        directory: Arc<ProcessIndexDirectory>,
        source: DataSource,
    },
}

impl HourProcessIndex<'_> {
    /// The seconds of the hour which have events of the process, in order. `None` - the
    /// hour is not indexed by process, or its index does not read: the trace scans it.
    pub async fn seconds_of(&self, process_id: i64) -> Option<Vec<usize>> {
        match self {
            HourProcessIndex::Full(index) => index.seconds_of(process_id),
            HourProcessIndex::Directory { directory, source } => {
                match directory.seconds_of(source, process_id).await {
                    Ok(seconds) => seconds,
                    Err(err) => {
                        println!("The process index does not read: {}", err);
                        None
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics_storage::{tests::metric, MICROS_PER_SECOND};

    /// 2023-11-14 22:00:00 UTC
    const HOUR: i64 = 1_699_999_200 * MICROS_PER_SECOND;

    fn at(second: i64) -> i64 {
        HOUR + second * MICROS_PER_SECOND
    }

    fn event(process_id: i64) -> Rec {
        metric(0, "billing", "charge", process_id)
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "my-telemetry-server-process-index-{}-{}",
            std::process::id(),
            name
        ))
    }

    async fn read_as_directory(index: &ProcessIndex, name: &str, process_ids: &[i64]) -> Vec<Option<Vec<usize>>> {
        let path = temp_file(name);
        std::fs::write(&path, index.to_file()).unwrap();

        let directory = HourProcessIndex::Directory {
            directory: Arc::new(ProcessIndexDirectory::read_local(HOUR, &path).unwrap()),
            source: DataSource::Local(path.clone()),
        };

        let mut result = Vec::new();
        for process_id in process_ids {
            result.push(directory.seconds_of(*process_id).await);
        }

        std::fs::remove_file(&path).unwrap();
        result
    }

    #[tokio::test]
    async fn the_seconds_of_a_process_are_the_one_its_id_names_and_the_ones_kept() {
        // Started in the middle of second 10, and goes on
        let long = at(10) + 500_000;
        // Over within the second it started in
        let short = at(20) + 7;
        // Not a time, as a writer over HTTP may give
        let not_a_time = 42;
        // Started in the hour before
        let earlier = HOUR - 5 * MICROS_PER_SECOND;

        let mut index = ProcessIndex::new(HOUR);

        index.add_records(10, &[event(long), event(long)]);
        index.add_records(11, &[event(long)]);
        index.add_records(15, &[event(long), event(not_a_time)]);
        index.add_records(20, &[event(short), event(short)]);
        index.add_records(3000, &[event(not_a_time)]);
        index.add_records(0, &[event(earlier)]);
        // The clock of the service is behind: before the process started
        index.add_records(19, &[event(short)]);

        assert_eq!(index.total_records(), 10);

        assert_eq!(index.seconds_of(long), Some(vec![10, 11, 15]));
        assert_eq!(index.seconds_of(short), Some(vec![19, 20]));
        assert_eq!(index.seconds_of(not_a_time), Some(vec![15, 3000]));
        assert_eq!(index.seconds_of(earlier), Some(vec![0]));

        // The second an id names is not kept: the id says it
        assert_eq!(index.pairs(), 6);

        // A process the hour has nothing of: the second it would have started in is all
        // there is to read - or nothing
        assert_eq!(index.seconds_of(at(100)), Some(vec![100]));
        assert_eq!(index.seconds_of(43), Some(vec![]));
        assert_eq!(index.seconds_of(-1), Some(vec![]));

        assert_eq!(ProcessIndex::from_file(HOUR, &index.to_file()), Some(index.clone()));

        let ids = [long, short, not_a_time, earlier, at(100), 43, -1, i64::MIN, i64::MAX];
        let read = read_as_directory(&index, "kept", &ids).await;

        for (process_id, seconds) in ids.iter().zip(read) {
            assert_eq!(seconds, index.seconds_of(*process_id), "process {}", process_id);
        }
    }

    #[tokio::test]
    async fn a_process_is_found_in_the_page_it_is_in() {
        let mut index = ProcessIndex::new(HOUR);

        // Ids which are not times, so every one of them is kept: 5 pages of pairs
        let ids: Vec<i64> = (0..10_000).map(|no| no * 3 - 9_000).collect();

        for (no, process_id) in ids.iter().enumerate() {
            index.add_records(no % SECONDS_PER_HOUR, &[event(*process_id)]);
            index.add_records((no * 7 + 1) % SECONDS_PER_HOUR, &[event(*process_id)]);
        }

        // One with events in most seconds of the hour: its pairs go over from page to page
        let everywhere = 6_000;
        for second in 0..3000 {
            index.add_records(second, &[event(everywhere)]);
        }

        assert!(index.pairs() > 5 * PAIRS_PER_PAGE);
        assert_eq!(index.seconds_of(everywhere).unwrap().len(), 3000);

        let asked: Vec<i64> = ids
            .iter()
            .step_by(37)
            .copied()
            .chain([
                ids[0],
                ids[ids.len() - 1],
                everywhere,
                // Between two of them, before the first, after the last
                everywhere + 1,
                ids[0] - 1,
                ids[ids.len() - 1] + 1,
                i64::MIN,
                i64::MAX,
            ])
            .collect();

        let read = read_as_directory(&index, "pages", &asked).await;

        for (process_id, seconds) in asked.iter().zip(read) {
            assert_eq!(seconds, index.seconds_of(*process_id), "process {}", process_id);
        }

        assert_eq!(ProcessIndex::from_file(HOUR, &index.to_file()), Some(index));
    }

    #[tokio::test]
    async fn an_hour_with_too_many_processes_is_not_indexed() {
        let mut index = ProcessIndex {
            max_pairs: 100,
            ..ProcessIndex::new(HOUR)
        };

        let records: Vec<Rec> = (0..101).map(event).collect();
        index.add_records(5, &records);
        index.add_records(6, &[event(1)]);

        assert_eq!(index.seconds_of(1), None);
        assert_eq!(index.pairs(), 0);
        assert_eq!(index.total_records(), 102);

        // And it stays so after the index is read back
        let read = ProcessIndex::from_file(HOUR, &index.to_file()).unwrap();
        assert_eq!(read.seconds_of(1), None);
        assert_eq!(read.total_records(), 102);

        assert_eq!(read_as_directory(&index, "let-go", &[1]).await, [None]);
    }

    /// How much of an hour the index takes: `cargo test process_index_size -- --ignored
    /// --nocapture`. 300 processes a second for ten minutes, four events each over tens of
    /// milliseconds - and one in fifty runs on for seconds.
    #[test]
    #[ignore]
    fn process_index_size() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let mut index = ProcessIndex::new(HOUR);
        let mut all_pairs = BTreeSet::new();
        let (mut processes, mut events) = (0u64, 0u64);

        for second in 0..600i64 {
            for _ in 0..300 {
                let process_id = at(second) + (next() % 1_000_000) as i64;

                let duration = if next() % 50 == 0 {
                    1_000_000 + (next() % 4_000_000) as i64
                } else {
                    10_000 + (next() % 150_000) as i64
                };

                processes += 1;

                for no in 0..4 {
                    let started = process_id + duration * no / 3;
                    let event_second = second_of(HOUR, started).unwrap();

                    index.add_records(event_second, &[event(process_id)]);
                    all_pairs.insert((process_id, event_second));
                    events += 1;
                }
            }
        }

        let file = index.to_file().len();

        println!(
            "{} events of {} processes: {} pairs kept of {} there are, {} KiB - {} KiB with the second each id names kept too",
            events,
            processes,
            index.pairs(),
            all_pairs.len(),
            file / 1024,
            all_pairs.len() * PAIR_LEN / 1024,
        );
    }

    #[tokio::test]
    async fn an_index_which_is_cut_does_not_read() {
        let mut index = ProcessIndex::new(HOUR);
        for process_id in 0..100 {
            index.add_records(7, &[event(process_id)]);
        }

        let file = index.to_file();

        assert!(ProcessIndex::from_file(HOUR, &file[..file.len() - 1]).is_none());
        assert!(ProcessIndex::from_file(HOUR, &file[..HEADER_LEN - 1]).is_none());
        assert!(ProcessIndex::from_file(HOUR, &[]).is_none());

        // Pairs out of order: not an index
        let mut swapped = file.clone();
        let pairs = Header::parse(&file).unwrap().directory_end();
        swapped.copy_within(pairs..pairs + PAIR_LEN, pairs + PAIR_LEN);
        assert!(ProcessIndex::from_file(HOUR, &swapped).is_none());

        let path = temp_file("cut");

        std::fs::write(&path, &file[..file.len() - 1]).unwrap();
        assert!(ProcessIndexDirectory::read_local(HOUR, &path).is_none());

        // The file is cut after its directory was read: the index says nothing
        std::fs::write(&path, &file).unwrap();
        let directory = HourProcessIndex::Directory {
            directory: Arc::new(ProcessIndexDirectory::read_local(HOUR, &path).unwrap()),
            source: DataSource::Local(path.clone()),
        };
        assert_eq!(directory.seconds_of(5).await, Some(vec![7]));

        std::fs::write(&path, &file[..pairs]).unwrap();
        assert_eq!(directory.seconds_of(5).await, None);

        std::fs::remove_file(&path).unwrap();
    }
}
