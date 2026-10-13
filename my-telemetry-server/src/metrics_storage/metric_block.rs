use prost::Message;
use rust_extensions::{ParseUInt32VariableSizeResult, UInt32VariableSize};

use crate::writer_grpc::TelemetryGrpcEvent;

use super::{encode_entry, ENTRY_TIME_LEN, MAX_ENTRY_BODY_LEN};

/// A record of the storage is the message which comes over the wire - there is no storage
/// proto. This is the only file of the storage which knows what a record is made of.
pub type Rec = TelemetryGrpcEvent;

const ZSTD_LEVEL: i32 = 3;

/// Records are put together into a block of about this many bytes before it is compressed.
/// my-logger-server takes 64 KiB; here the timestamps and the ids of an event do not
/// compress, a block shrinks about 4 times (`compression_ratio` below measures it), and it
/// has to fit an entry of 8 KiB - a block which does not is compressed again in halves.
const RAW_BLOCK_LEN: usize = 24 * 1024;

const MAX_PAYLOAD_LEN: usize = MAX_ENTRY_BODY_LEN - ENTRY_TIME_LEN;

/// What a string cut to fit an entry ends with
const TRUNCATED_MARK: &str = "...[truncated]";

/// A string shorter than this is never cut: the number of tags is
const MIN_LEN_TO_CUT: usize = 64;

/// The tags a search filters on. They are the last thing a record loses.
const KEPT_TAGS: [&str; 2] = [crate::mappers::CLIENT_ID_TAG, "user_id"];

/// The time a record is kept by: the moment its event started, unix microseconds
pub fn time_of(record: &Rec) -> i64 {
    record.started_at
}

/// An entry: a block of records compressed to fit it
pub struct CompressedBlock {
    /// Its time is the time of its first record
    pub entry: Vec<u8>,
    /// How many records it holds
    pub records: usize,
}

/// The records of one second, in time order, as entries: blocks of them, each
/// compressed to fit an entry. The records are only read: the caller keeps them for the
/// key index, and compresses a second before it takes the lock of its hour.
pub fn compress_records(records: &[Rec]) -> Vec<CompressedBlock> {
    let mut result = Vec::new();
    let mut from = 0;
    let mut block_len = 0;

    for (index, record) in records.iter().enumerate() {
        let record_len = record.encoded_len();

        if index > from && block_len + record_len > RAW_BLOCK_LEN {
            compress_block(&records[from..index], &mut result);
            from = index;
            block_len = 0;
        }

        block_len += record_len;
    }

    if from < records.len() {
        compress_block(&records[from..], &mut result);
    }

    result
}

/// A block which does not compress to fit an entry is halved; a record which does not
/// fit alone is cut.
fn compress_block(records: &[Rec], out: &mut Vec<CompressedBlock>) {
    if let Some(entry) = compress_to_entry(records) {
        out.push(CompressedBlock {
            entry,
            records: records.len(),
        });
        return;
    }

    if records.len() > 1 {
        let (lower, upper) = records.split_at(records.len() / 2);
        compress_block(lower, out);
        compress_block(upper, out);
        return;
    }

    // A copy of it is cut until it fits: the record of the caller stays as it came
    let mut record = records[0].clone();

    while shrink(&mut record) {
        if let Some(entry) = compress_to_entry(std::slice::from_ref(&record)) {
            out.push(CompressedBlock { entry, records: 1 });
            return;
        }
    }

    println!(
        "A metric of {} / {} is dropped: it does not fit an entry and has nothing left to cut",
        record.service_name, record.event_data
    );
}

/// `None` - the records do not compress to fit an entry
fn compress_to_entry(records: &[Rec]) -> Option<Vec<u8>> {
    let payload = zstd::bulk::compress(&encode_records(records), ZSTD_LEVEL)
        .expect("zstd compresses a buffer in memory");

    (payload.len() <= MAX_PAYLOAD_LEN).then(|| encode_entry(time_of(&records[0]), &payload))
}

fn encode_records(records: &[Rec]) -> Vec<u8> {
    let mut result = Vec::new();

    for record in records {
        UInt32VariableSize::new(record.encoded_len() as u32).serialize(&mut result);
        record
            .encode(&mut result)
            .expect("a Vec grows to hold a record");
    }

    result
}

/// Cuts the longest string of the record in half - the free text first, the names a search
/// goes by only when nothing else is long. With only short strings left, drops half of the
/// tags, the ones a search filters on last. `false` - there is nothing left to cut.
fn shrink(record: &mut Rec) -> bool {
    let fail_len = record.fail.as_ref().map_or(0, |value| value.len());
    let success_len = record.success.as_ref().map_or(0, |value| value.len());
    let longest_tag = record
        .tags
        .iter()
        .enumerate()
        .max_by_key(|(_, tag)| tag.value.len())
        .map(|(index, tag)| (index, tag.value.len()));
    let tag_len = longest_tag.map_or(0, |(_, len)| len);

    let longest_text = fail_len.max(success_len).max(tag_len);

    if longest_text >= MIN_LEN_TO_CUT {
        if fail_len == longest_text {
            cut_in_half(record.fail.as_mut().unwrap());
        } else if success_len == longest_text {
            cut_in_half(record.success.as_mut().unwrap());
        } else {
            let (index, _) = longest_tag.unwrap();
            cut_in_half(&mut record.tags[index].value);
        }

        return true;
    }

    let longest_name = record.event_data.len().max(record.service_name.len());

    if longest_name >= MIN_LEN_TO_CUT {
        if record.event_data.len() == longest_name {
            cut_in_half(&mut record.event_data);
        } else {
            cut_in_half(&mut record.service_name);
        }

        return true;
    }

    let droppable = record
        .tags
        .iter()
        .filter(|tag| !KEPT_TAGS.contains(&tag.key.as_str()))
        .count();

    if droppable > 0 {
        let mut to_drop = droppable.div_ceil(2);

        // From the end: the tags which came last go first
        let mut index = record.tags.len();
        while to_drop > 0 {
            index -= 1;
            if !KEPT_TAGS.contains(&record.tags[index].key.as_str()) {
                record.tags.remove(index);
                to_drop -= 1;
            }
        }

        return true;
    }

    false
}

fn cut_in_half(value: &mut String) {
    let len = value
        .strip_suffix(TRUNCATED_MARK)
        .map(|value| value.len())
        .unwrap_or(value.len());

    let mut at = len / 2;
    while !value.is_char_boundary(at) {
        at -= 1;
    }

    value.truncate(at);
    value.push_str(TRUNCATED_MARK);
}

pub fn decompress_records(payload: &[u8]) -> Result<Vec<Rec>, String> {
    let raw = decompress(payload)?;

    let mut result = Vec::new();

    for record in RawRecords::new(&raw) {
        let record = Rec::decode(record?)
            .map_err(|err| format!("A record of a block does not decode: {}", err))?;

        result.push(record);
    }

    Ok(result)
}

fn decompress(payload: &[u8]) -> Result<Vec<u8>, String> {
    zstd::stream::decode_all(payload).map_err(|err| format!("A block does not decompress: {}", err))
}

/// The records of a decompressed block, each still encoded
struct RawRecords<'s> {
    raw: &'s [u8],
    offset: usize,
}

impl<'s> RawRecords<'s> {
    fn new(raw: &'s [u8]) -> Self {
        Self { raw, offset: 0 }
    }
}

impl<'s> Iterator for RawRecords<'s> {
    type Item = Result<&'s [u8], String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.raw.len() {
            return None;
        }

        let (len, prefix_len) = match UInt32VariableSize::from_slice(&self.raw[self.offset..]) {
            ParseUInt32VariableSizeResult::Ok { value, size } => {
                (value.get_value() as usize, size)
            }
            ParseUInt32VariableSizeResult::NotEnoughDataInBuffer(_) => {
                self.offset = self.raw.len();
                return Some(Err("The length of a record of a block is cut".to_string()));
            }
        };

        let start = self.offset + prefix_len;
        let end = start + len;

        if end > self.raw.len() {
            self.offset = self.raw.len();
            return Some(Err("A record reaches past the end of its block".to_string()));
        }

        self.offset = end;

        Some(Ok(&self.raw[start..end]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics_storage::parse_entry;
    use crate::writer_grpc::EventGrpcTag;

    fn tag(key: &str, value: &str) -> EventGrpcTag {
        EventGrpcTag {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn record(started_at: i64, fail: Option<String>) -> Rec {
        Rec {
            process_id: started_at - 1500,
            started_at,
            finished_at: started_at + 2500,
            service_name: "trading-api".to_string(),
            event_data: format!("POST /api/v1/orders/{}", started_at % 7),
            success: if fail.is_none() {
                Some("Ok".to_string())
            } else {
                None
            },
            fail,
            tags: vec![tag("ip", "10.0.0.7"), tag("client_id", "client-17")],
        }
    }

    fn records_of(blocks: &[CompressedBlock]) -> Vec<Rec> {
        blocks
            .iter()
            .flat_map(|block| {
                let entry = parse_entry(&block.entry, 0).unwrap();
                let records = decompress_records(entry.payload).unwrap();
                assert_eq!(records.len(), block.records);
                records
            })
            .collect()
    }

    /// Poorly compressible text: random letters
    fn random_text(len: usize) -> String {
        let mut state = 0x2545_F491_4F6C_DD1Du64;

        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                char::from(b'a' + (state % 26) as u8)
            })
            .collect()
    }

    #[test]
    fn records_come_back_from_their_blocks() {
        let records: Vec<Rec> = (0..5000)
            .map(|no| record(1_700_000_000_000_000 + no * 173, None))
            .collect();

        let blocks = compress_records(&records);

        assert!(blocks.len() > 1);
        assert!(blocks
            .iter()
            .all(|block| block.entry.len() <= MAX_ENTRY_BODY_LEN + 2));
        assert_eq!(
            parse_entry(&blocks[0].entry, 0).unwrap().time,
            1_700_000_000_000_000
        );
        assert_eq!(records_of(&blocks), records);
    }

    #[test]
    fn a_record_too_long_for_an_entry_is_cut_and_keeps_what_a_search_goes_by() {
        let fail = random_text(100_000);

        let mut long = record(1, Some(fail.clone()));
        long.tags.push(tag("trace", &random_text(300)));

        let blocks = compress_records(std::slice::from_ref(&long));
        let records = records_of(&blocks);

        assert_eq!(records.len(), 1);

        let stored_fail = records[0].fail.as_ref().unwrap();
        assert!(stored_fail.ends_with(TRUNCATED_MARK));
        assert!(fail.starts_with(stored_fail.strip_suffix(TRUNCATED_MARK).unwrap()));

        assert_eq!(records[0].service_name, long.service_name);
        assert_eq!(records[0].event_data, long.event_data);
        assert!(records[0]
            .tags
            .iter()
            .any(|tag| tag.key == "client_id" && tag.value == "client-17"));
    }

    #[test]
    fn tags_are_dropped_before_the_client_id_is() {
        let mut record = record(1, None);
        record.tags = vec![
            tag("a", "1"),
            tag("client_id", "client-17"),
            tag("b", "2"),
            tag("c", "3"),
        ];

        // Nothing is long enough to cut: half of the other tags go, the last ones first
        assert!(shrink(&mut record));
        let keys: Vec<&str> = record.tags.iter().map(|tag| tag.key.as_str()).collect();
        assert_eq!(keys, ["a", "client_id"]);

        assert!(shrink(&mut record));
        let keys: Vec<&str> = record.tags.iter().map(|tag| tag.key.as_str()).collect();
        assert_eq!(keys, ["client_id"]);

        assert!(!shrink(&mut record));
    }

    /// Prints how a second of telemetry compresses - what `RAW_BLOCK_LEN` is picked by.
    /// `cargo test compression_ratio -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn compression_ratio() {
        let services = ["trading-api", "session-manager", "quotes-feed", "billing"];
        let actions = [
            "POST /api/v1/orders",
            "GET /api/v1/positions",
            "sql: select * from accounts where id = $1",
            "grpc: SessionManager/GetSession",
        ];

        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let hour = 1_700_000_000_000_000i64;

        let records: Vec<Rec> = (0..2000)
            .map(|no| {
                let started_at = hour + no * 500 + (next() % 400) as i64;

                Rec {
                    process_id: started_at - (next() % 2_000_000) as i64,
                    started_at,
                    finished_at: started_at + (next() % 50_000) as i64,
                    service_name: services[(next() % 4) as usize].to_string(),
                    event_data: actions[(next() % 4) as usize].to_string(),
                    success: Some("Ok".to_string()),
                    fail: None,
                    tags: vec![
                        tag("client_id", &format!("client-{}", next() % 5000)),
                        tag("ip", &format!("10.0.{}.{}", next() % 255, next() % 255)),
                    ],
                }
            })
            .collect();

        let raw: usize = records.iter().map(|record| record.encoded_len() + 1).sum();
        let blocks = compress_records(&records);
        let compressed: usize = blocks.iter().map(|block| block.entry.len()).sum();

        println!(
            "{} records: {} bytes raw, {} compressed in {} blocks - {:.1} times; the largest entry is {} bytes",
            2000,
            raw,
            compressed,
            blocks.len(),
            raw as f64 / compressed as f64,
            blocks.iter().map(|block| block.entry.len()).max().unwrap()
        );
    }
}
