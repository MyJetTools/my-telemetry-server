use std::{collections::BTreeMap, sync::Arc};

use parking_lot::Mutex;
use rust_extensions::{ParseUInt32VariableSizeResult, UInt32VariableSize};

use super::{read_bytes, DataSource, Rec, READ_LEN, SECONDS_PER_HOUR};

/// The key the records of each service are counted under, whatever their action: its
/// values are the names of the services. No service is named like this.
const SERVICES_KEY: &str = "\0";

/// A service with more actions than this in an hour is not indexed by action in it: ids
/// put into the names of the actions - such an index would grow as large as the data. Its
/// records are still counted as a whole.
const MAX_ACTIONS_PER_SERVICE: usize = 4096;

/// With more services than this in an hour nothing is indexed in it
const MAX_SERVICES: usize = 1024;

/// With more `(value, second)` pairs than this the index of the hour is let go: it is in
/// memory whole for the hours which are written, 8 bytes a pair
const MAX_PAIRS: usize = 4_000_000;

/// `total_records u64 · directory_len u32 · format u8`
const HEADER_LEN: usize = 13;

/// Seconds packed: varints of the step from the second before and of the records
const FORMAT: u8 = 1;

/// How many records of each second each action of each service has - what the one search
/// by service and action goes by. The key is the service, the value is the action; the
/// names are kept as they are written, a search matches them exactly.
///
/// The file is the `.index-by-key` of my-logger-server byte for byte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyIndex {
    /// service → action → (second, records), by second. Under [`SERVICES_KEY`]: service →
    /// (second, records of the service). A service with no action is one with too many.
    keys: BTreeMap<String, BTreeMap<String, Vec<(u16, u32)>>>,
    /// The records counted: the counts of the index of the hour add up to it when the two
    /// agree
    total_records: u64,
    pairs: usize,
}

impl KeyIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total_records(&self) -> u64 {
        self.total_records
    }

    /// So many of the records counted were not written - they fit no entry: the hour has
    /// that many fewer. What is counted of their actions stays: an upper bound.
    pub fn forget_records(&mut self, records: usize) {
        self.total_records = self.total_records.saturating_sub(records as u64);
    }

    /// Nothing is indexed in the hour any more: it has too much to index
    fn is_let_go(&self) -> bool {
        self.keys.is_empty() && self.total_records > 0
    }

    pub fn add_records(&mut self, second: usize, records: &[Rec]) {
        let let_go = self.is_let_go();

        self.total_records += records.len() as u64;

        if let_go {
            return;
        }

        for record in records {
            if !self.add_record(second, record) {
                println!("The hour has too much to index by service and action: its searches scan it");
                self.keys.clear();
                self.pairs = 0;
                return;
            }
        }
    }

    /// `false` - the index has outgrown what an hour is indexed within
    fn add_record(&mut self, second: usize, record: &Rec) -> bool {
        let service = record.service_name.as_str();

        if !self.keys.contains_key(SERVICES_KEY) {
            self.keys.insert(SERVICES_KEY.to_string(), BTreeMap::new());
        }

        let services = self.keys.get_mut(SERVICES_KEY).unwrap();
        let is_new_service = !services.contains_key(service);

        if is_new_service && services.len() >= MAX_SERVICES {
            return false;
        }

        self.pairs += count_in(services, service, second);

        // The name of the key which counts the services can not be a key of its own
        if service != SERVICES_KEY {
            if is_new_service {
                self.keys.insert(service.to_string(), BTreeMap::new());
            }

            let actions = self.keys.get_mut(service).unwrap();

            // No actions: the service has had too many of them in this hour
            if !is_new_service && actions.is_empty() {
                return self.pairs <= MAX_PAIRS;
            }

            let action = record.event_data.as_str();

            if !actions.contains_key(action) && actions.len() >= MAX_ACTIONS_PER_SERVICE {
                self.pairs -= actions.values().map(Vec::len).sum::<usize>();
                actions.clear();
            } else {
                self.pairs += count_in(actions, action, second);
            }
        }

        self.pairs <= MAX_PAIRS
    }

    /// The records of the second with the action of the service, as a search is told
    #[cfg(test)]
    pub fn count(&self, service: &str, action: &str, second: usize) -> Option<u32> {
        let seconds = match find(&self.keys, service, action)? {
            Found::None => return Some(0),
            Found::Seconds(key, value) => &self.keys[key][value],
        };

        Some(
            seconds
                .binary_search_by_key(&(second as u16), |(second, _)| *second)
                .map_or(0, |index| seconds[index].1),
        )
    }

    /// `total_records u64 · directory_len u32 · format u8`, then the directory - for each
    /// key its name and for each value of it the value, and the offset of its seconds from
    /// the end of the directory and their length, varints both - then the seconds of every
    /// value, packed: `varint(second - the second before) · varint(records)` each, the
    /// first second stepped from 0.
    pub fn to_file(&self) -> Vec<u8> {
        let mut directory = Vec::new();
        let mut seconds_bytes = Vec::new();

        write_len(&mut directory, self.keys.len());

        for (key, by_value) in &self.keys {
            write_string(&mut directory, key);
            write_len(&mut directory, by_value.len());

            for (value, seconds) in by_value {
                let offset = seconds_bytes.len();
                pack_seconds(seconds, &mut seconds_bytes);

                write_string(&mut directory, value);
                write_len(&mut directory, offset);
                write_len(&mut directory, seconds_bytes.len() - offset);
            }
        }

        let mut result = Vec::with_capacity(HEADER_LEN + directory.len() + seconds_bytes.len());
        result.extend_from_slice(&self.total_records.to_le_bytes());
        result.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        result.push(FORMAT);
        result.extend_from_slice(&directory);
        result.extend_from_slice(&seconds_bytes);
        result
    }

    /// `None` when the bytes are not an index of this format
    pub fn from_file(bytes: &[u8]) -> Option<Self> {
        let end = KeyIndexDirectory::end_of(bytes)?;
        let total_records = u64::from_le_bytes(bytes[0..8].try_into().unwrap());

        let directory = bytes.get(HEADER_LEN..end)?;
        let seconds_bytes = &bytes[end..];

        let mut reader = Reader {
            bytes: directory,
            position: 0,
        };

        let mut keys = BTreeMap::new();
        let mut pairs = 0;

        for _ in 0..reader.read_len()? {
            let key = reader.read_string()?;
            let mut by_value = BTreeMap::new();

            for _ in 0..reader.read_len()? {
                let value = reader.read_string()?;
                let offset = reader.read_len()?;
                let len = reader.read_len()?;

                let block = seconds_bytes.get(offset..offset.checked_add(len)?)?;
                let seconds = unpack_seconds(block)?;
                pairs += seconds.len();
                by_value.insert(value, seconds);
            }

            keys.insert(key, by_value);
        }

        Some(Self {
            keys,
            total_records,
            pairs,
        })
    }
}

/// One more record of the value in the second. Returns how many pairs it added: 1 or 0.
fn count_in(by_value: &mut BTreeMap<String, Vec<(u16, u32)>>, value: &str, second: usize) -> usize {
    // A name is allocated only the first time it is met
    if !by_value.contains_key(value) {
        by_value.insert(value.to_string(), Vec::new());
    }

    let seconds = by_value.get_mut(value).unwrap();

    match seconds.binary_search_by_key(&(second as u16), |(second, _)| *second) {
        Ok(index) => {
            seconds[index].1 += 1;
            0
        }
        Err(index) => {
            seconds.insert(index, (second as u16, 1));
            1
        }
    }
}

/// Where the counts of a search by service and action are
enum Found<'s> {
    /// The hour has no records of the service
    None,
    /// The seconds of this value of this key
    Seconds(&'s str, &'s str),
}

/// `None` - the hour does not index: every second may match. The counts of the action,
/// or of the whole service when it has too many actions to index: an upper bound then.
fn find<'s, T>(
    keys: &'s BTreeMap<String, BTreeMap<String, T>>,
    service: &'s str,
    action: &'s str,
) -> Option<Found<'s>> {
    let services = keys.get(SERVICES_KEY)?;

    if !services.contains_key(service) {
        return Some(Found::None);
    }

    let actions = keys
        .get(service)
        .filter(|actions| service != SERVICES_KEY && !actions.is_empty());

    Some(match actions {
        None => Found::Seconds(SERVICES_KEY, service),
        Some(actions) if actions.contains_key(action) => Found::Seconds(service, action),
        Some(_) => Found::None,
    })
}

/// Where the packed seconds of a value are in the file of a key index, and their length
#[derive(Debug, Clone, Copy)]
struct SecondsBlock {
    offset: u64,
    len: u32,
}

/// The key index of an hour as a search reads it, from a file or from S3 alike: its page
/// of every key and value in memory, the seconds of a value read when a search asks for it
/// - 64 KiB a read at least, the blocks close to each other in one - and kept.
pub struct KeyIndexDirectory {
    keys: BTreeMap<String, BTreeMap<String, SecondsBlock>>,
    total_records: u64,
    loaded: Mutex<BTreeMap<(String, String), Arc<Vec<(u16, u32)>>>>,
    /// What the read of the directory brought in past it: the seconds at the start of the
    /// file, read with the directory - the whole index, when it is shorter than one read
    read_ahead: Vec<u8>,
    /// Where `read_ahead` is in the file
    read_ahead_start: u64,
}

impl KeyIndexDirectory {
    /// Nothing indexed: every search of the hour scans it
    pub fn empty() -> Self {
        Self {
            keys: BTreeMap::new(),
            total_records: 0,
            loaded: Mutex::new(BTreeMap::new()),
            read_ahead: Vec::new(),
            read_ahead_start: 0,
        }
    }

    pub fn total_records(&self) -> u64 {
        self.total_records
    }

    /// Where the directory ends in the file: read off its header. `None` while the header
    /// is cut, or when it is not of this format.
    fn end_of(header: &[u8]) -> Option<usize> {
        if header.len() < HEADER_LEN || header[12] != FORMAT {
            return None;
        }

        let directory_len = u32::from_le_bytes(header[8..12].try_into().unwrap());
        Some(HEADER_LEN + directory_len as usize)
    }

    /// `bytes` start with the header and hold the directory whole; what follows it is kept
    fn parse(bytes: &[u8]) -> Option<Self> {
        let total_records = u64::from_le_bytes(bytes.get(0..8)?.try_into().unwrap());
        let end = Self::end_of(bytes)?;

        let mut reader = Reader {
            bytes: bytes.get(HEADER_LEN..end)?,
            position: 0,
        };

        let mut keys = BTreeMap::new();

        for _ in 0..reader.read_len()? {
            let key = reader.read_string()?;
            let mut by_value = BTreeMap::new();

            for _ in 0..reader.read_len()? {
                let value = reader.read_string()?;
                let offset = reader.read_len()? as u64;
                let len = reader.read_len()? as u32;

                by_value.insert(
                    value,
                    SecondsBlock {
                        offset: end as u64 + offset,
                        len,
                    },
                );
            }

            keys.insert(key, by_value);
        }

        Some(Self {
            keys,
            total_records,
            loaded: Mutex::new(BTreeMap::new()),
            read_ahead: bytes[end..].to_vec(),
            read_ahead_start: end as u64,
        })
    }

    /// The seconds of the block, when the read of the directory brought them in
    fn read_ahead_of(&self, block: &SecondsBlock) -> Option<&[u8]> {
        let from = usize::try_from(block.offset.checked_sub(self.read_ahead_start)?).ok()?;
        self.read_ahead.get(from..from + block.len as usize)
    }

    /// The directory of a local file, read the same way; `None` when it does not read
    pub fn read_local(path: &std::path::Path) -> Option<Self> {
        use std::io::Read;

        let mut file = std::fs::File::open(path).ok()?;
        let mut bytes = vec![0u8; READ_LEN];
        let mut read = 0;

        while read < bytes.len() {
            match file.read(&mut bytes[read..]).ok()? {
                0 => break,
                size => read += size,
            }
        }

        bytes.truncate(read);

        let end = Self::end_of(&bytes)?;

        if end > bytes.len() {
            let start = bytes.len();
            bytes.resize(end, 0);
            file.read_exact(&mut bytes[start..]).ok()?;
        }

        Self::parse(&bytes)
    }

    /// The directory of the file: one read of 64 KiB, and the rest of it when it is longer
    pub async fn read(source: &DataSource) -> Result<Self, String> {
        let mut bytes = read_bytes(source, 0, READ_LEN).await?;

        let end = Self::end_of(&bytes).ok_or_else(|| "The key index is cut".to_string())?;

        if end > bytes.len() {
            let rest = read_bytes(source, bytes.len() as u64, end - bytes.len()).await?;
            bytes.extend_from_slice(&rest);
        }

        Self::parse(&bytes).ok_or_else(|| "The directory of the key index does not read".to_string())
    }

    /// The seconds of the value of the key. They are taken from what the read of the
    /// directory brought in, or read - 64 KiB at least - and kept.
    async fn seconds_of(
        &self,
        source: &DataSource,
        key: &str,
        value: &str,
    ) -> Result<Arc<Vec<(u16, u32)>>, String> {
        let cache_key = (key.to_string(), value.to_string());

        if let Some(seconds) = self.loaded.lock().get(&cache_key) {
            return Ok(seconds.clone());
        }

        let Some(block) = self.keys.get(key).and_then(|by_value| by_value.get(value)) else {
            return Ok(Arc::default());
        };

        let seconds = match self.read_ahead_of(block) {
            Some(bytes) => unpack_seconds(bytes).ok_or_else(seconds_do_not_read)?,
            None => {
                let len = (block.len as usize).max(READ_LEN);
                let bytes = read_bytes(source, block.offset, len).await?;

                let block_bytes = bytes
                    .get(..block.len as usize)
                    .ok_or_else(|| "The key index ends inside the seconds of a value".to_string())?;

                unpack_seconds(block_bytes).ok_or_else(seconds_do_not_read)?
            }
        };

        let seconds = Arc::new(seconds);
        self.loaded.lock().insert(cache_key, seconds.clone());

        Ok(seconds)
    }
}

fn seconds_do_not_read() -> String {
    "The seconds of a value of the key index do not read".to_string()
}

/// `varint(second - the second before) · varint(records)` for each second, by second
fn pack_seconds(seconds: &[(u16, u32)], out: &mut Vec<u8>) {
    let mut before = 0;

    for (second, records) in seconds {
        write_len(out, (second - before) as usize);
        write_len(out, *records as usize);
        before = *second;
    }
}

fn unpack_seconds(bytes: &[u8]) -> Option<Vec<(u16, u32)>> {
    let mut reader = Reader { bytes, position: 0 };
    let mut result = Vec::new();
    let mut second = 0;

    while reader.position < bytes.len() {
        second += reader.read_len()?;
        let records = reader.read_len()? as u32;

        if second >= SECONDS_PER_HOUR {
            return None;
        }

        result.push((second as u16, records));
    }

    Some(result)
}

/// The key index of an hour as a search uses it
#[derive(Clone)]
pub enum HourKeyIndex {
    /// The hour is written into: the whole index in memory, kept up to date by the writes
    Full(Arc<KeyIndex>),
    /// The hour is read: the directory in memory, the seconds read from `source` when asked
    Directory {
        directory: Arc<KeyIndexDirectory>,
        source: DataSource,
    },
}

impl HourKeyIndex {
    /// How many records of each second the action of the service has. When the hour does
    /// not index, or the index does not read, it narrows nothing: the search scans.
    pub async fn counts_of(&self, service: &str, action: &str) -> FilterCounts {
        match self.seconds_of(service, action).await {
            Ok(Some(seconds)) => {
                let mut counts = vec![0u32; SECONDS_PER_HOUR];

                for (second, records) in seconds.iter() {
                    counts[*second as usize] += records;
                }

                FilterCounts {
                    counts: Some(counts),
                }
            }
            Ok(None) => FilterCounts::unknown(),
            Err(err) => {
                println!("The key index does not read: {}", err);
                FilterCounts::unknown()
            }
        }
    }

    async fn seconds_of(
        &self,
        service: &str,
        action: &str,
    ) -> Result<Option<Arc<Vec<(u16, u32)>>>, String> {
        match self {
            HourKeyIndex::Full(index) => Ok(find(&index.keys, service, action).map(|found| {
                match found {
                    Found::None => Arc::default(),
                    Found::Seconds(key, value) => Arc::new(index.keys[key][value].clone()),
                }
            })),
            HourKeyIndex::Directory { directory, source } => {
                match find(&directory.keys, service, action) {
                    None => Ok(None),
                    Some(Found::None) => Ok(Some(Arc::default())),
                    Some(Found::Seconds(key, value)) => {
                        Ok(Some(directory.seconds_of(source, key, value).await?))
                    }
                }
            }
        }
    }
}

/// What an index says of a search, second by second: the key index of one by service and
/// action, the process index of a trace
pub struct FilterCounts {
    /// `None` - the index says nothing: every record may match
    counts: Option<Vec<u32>>,
}

impl FilterCounts {
    /// Of a search no index serves: every record may match
    pub fn unknown() -> Self {
        Self { counts: None }
    }

    /// Only the records of these seconds may match - any number of them
    pub fn of_seconds(seconds: &[usize]) -> Self {
        let mut counts = vec![0u32; SECONDS_PER_HOUR];

        for second in seconds {
            counts[*second] = u32::MAX;
        }

        Self {
            counts: Some(counts),
        }
    }

    /// The index names the seconds to read
    pub fn is_known(&self) -> bool {
        self.counts.is_some()
    }

    /// At most how many of the `records` of the second match
    pub fn estimate(&self, records: u32, second: usize) -> u32 {
        match &self.counts {
            Some(counts) => records.min(counts[second]),
            None => records,
        }
    }
}

fn write_len(out: &mut Vec<u8>, len: usize) {
    UInt32VariableSize::new(len as u32).serialize(out);
}

fn write_string(out: &mut Vec<u8>, value: &str) {
    write_len(out, value.len());
    out.extend_from_slice(value.as_bytes());
}

struct Reader<'s> {
    bytes: &'s [u8],
    position: usize,
}

impl<'s> Reader<'s> {
    fn read_len(&mut self) -> Option<usize> {
        let rest = self.bytes.get(self.position..).filter(|rest| !rest.is_empty())?;

        match UInt32VariableSize::from_slice(rest) {
            ParseUInt32VariableSizeResult::Ok { value, size } => {
                self.position += size;
                Some(value.get_value() as usize)
            }
            ParseUInt32VariableSizeResult::NotEnoughDataInBuffer(_) => None,
        }
    }

    fn read_string(&mut self) -> Option<String> {
        let len = self.read_len()?;
        let bytes = self.bytes.get(self.position..self.position + len)?;
        self.position += len;
        String::from_utf8(bytes.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(service: &str, action: &str) -> Rec {
        Rec {
            process_id: 1,
            started_at: 0,
            finished_at: 1,
            service_name: service.to_string(),
            event_data: action.to_string(),
            success: None,
            fail: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn counts_the_actions_of_the_services() {
        let mut index = KeyIndex::new();

        index.add_records(
            5,
            &[
                record("billing", "charge"),
                record("billing", "charge"),
                record("billing", "refund"),
            ],
        );
        index.add_records(7, &[record("trading", "charge")]);

        assert_eq!(index.count("billing", "charge", 5), Some(2));
        assert_eq!(index.count("billing", "refund", 5), Some(1));
        assert_eq!(index.count("billing", "charge", 7), Some(0));
        // The same name of an action in another service is another action
        assert_eq!(index.count("trading", "charge", 7), Some(1));
        assert_eq!(index.count("trading", "charge", 5), Some(0));
        // Names are matched as they are written
        assert_eq!(index.count("Billing", "charge", 5), Some(0));
        assert_eq!(index.count("billing", "Charge", 5), Some(0));
        // A service the hour has not got
        assert_eq!(index.count("quotes", "charge", 5), Some(0));
        assert_eq!(index.total_records(), 4);

        assert_eq!(KeyIndex::from_file(&index.to_file()), Some(index));
    }

    #[test]
    fn an_hour_with_no_records_says_nothing() {
        let index = KeyIndex::new();
        assert_eq!(index.count("billing", "charge", 5), None);
    }

    #[tokio::test]
    async fn seconds_past_the_first_read_are_read_from_the_file() {
        let mut index = KeyIndex::new();

        // 900 actions in each of 120 seconds: their seconds take more than two reads
        for second in 0..120 {
            let records: Vec<Rec> = (0..900)
                .map(|no| record("billing", &no.to_string()))
                .collect();
            index.add_records(second, &records);
        }

        let path = std::env::temp_dir().join(format!(
            "my-telemetry-server-key-index-{}",
            std::process::id()
        ));
        std::fs::write(&path, index.to_file()).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 2 * READ_LEN as u64);

        let key_index = HourKeyIndex::Directory {
            directory: Arc::new(KeyIndexDirectory::read_local(&path).unwrap()),
            source: DataSource::Local(path.clone()),
        };

        // The first one is in the read of the page, the others past it
        for action in ["0", "450", "899"] {
            let counts = key_index.counts_of("billing", action).await;

            for second in 0..120 {
                assert_eq!(counts.estimate(u32::MAX, second), 1, "action {}", action);
            }
            assert_eq!(counts.estimate(u32::MAX, 120), 0);
        }

        // Neither the service nor the action is in the hour: nothing to read
        assert_eq!(key_index.counts_of("billing", "none").await.estimate(9, 5), 0);
        assert_eq!(key_index.counts_of("quotes", "0").await.estimate(9, 5), 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn seconds_are_packed() {
        let mut index = KeyIndex::new();

        // A service writing every second of the hour, up to 9 records a second
        for second in 0..SECONDS_PER_HOUR {
            let records: Vec<Rec> = (0..1 + second % 9)
                .map(|_| record("billing", "charge"))
                .collect();
            index.add_records(second, &records);
        }

        // Two values - the service and its action - in 2 bytes a second each, not 6
        let file = index.to_file();
        assert!(file.len() < 2 * SECONDS_PER_HOUR * 2 + 100, "{} bytes", file.len());
        assert_eq!(KeyIndex::from_file(&file), Some(index));
    }

    #[test]
    fn a_service_with_too_many_actions_is_counted_as_a_whole() {
        let mut index = KeyIndex::new();

        let records: Vec<Rec> = (0..MAX_ACTIONS_PER_SERVICE + 1)
            .map(|no| record("billing", &format!("GET /orders/{}", no)))
            .collect();
        index.add_records(1, &records);
        index.add_records(2, &[record("billing", "GET /orders/1"), record("trading", "buy")]);

        // Not by action any more: every record of the service may be the one
        assert_eq!(
            index.count("billing", "GET /orders/1", 1),
            Some(records.len() as u32)
        );
        assert_eq!(index.count("billing", "GET /orders/1", 2), Some(1));
        assert_eq!(index.count("billing", "GET /orders/1", 3), Some(0));
        // The other services are indexed as they were
        assert_eq!(index.count("trading", "buy", 2), Some(1));
        assert_eq!(index.count("trading", "sell", 2), Some(0));

        // And it stays so after the index is read back
        let read = KeyIndex::from_file(&index.to_file()).unwrap();
        assert_eq!(read.count("billing", "GET /orders/1", 2), Some(1));
        assert_eq!(read, index);
    }

    #[test]
    fn an_hour_with_too_many_services_is_not_indexed() {
        let mut index = KeyIndex::new();

        let records: Vec<Rec> = (0..MAX_SERVICES + 1)
            .map(|no| record(&format!("service-{}", no), "action"))
            .collect();
        index.add_records(1, &records);
        index.add_records(2, &[record("service-1", "action")]);

        assert_eq!(index.count("service-1", "action", 1), None);
        assert_eq!(index.total_records(), records.len() as u64 + 1);

        let read = KeyIndex::from_file(&index.to_file()).unwrap();
        assert_eq!(read.count("service-1", "action", 2), None);
        assert_eq!(read.total_records(), index.total_records());
    }
}
