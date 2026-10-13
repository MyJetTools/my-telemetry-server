use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::writer_grpc::EventGrpcTag;

use super::*;

/// 2023-11-14 22:00:00 UTC - exactly on an hour boundary
pub const HOUR_START: i64 = 1_699_999_200 * MICROS_PER_SECOND;

pub fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "my-telemetry-server-tests-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Letters which compress poorly, so blocks are KiBs and frames fill and get halved
pub struct Random(pub u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, max: u64) -> u64 {
        self.next() % max
    }

    pub fn text(&mut self, len: usize) -> String {
        (0..len)
            .map(|_| char::from(b'a' + self.below(26) as u8))
            .collect()
    }
}

pub fn metric(started_at: i64, service: &str, action: &str, process_id: i64) -> Rec {
    Rec {
        process_id,
        started_at,
        finished_at: started_at + 100,
        service_name: service.to_string(),
        event_data: action.to_string(),
        success: Some("ok".to_string()),
        fail: None,
        tags: vec![EventGrpcTag {
            key: "client_id".to_string(),
            value: "client-1".to_string(),
        }],
    }
}

fn record(random: &mut Random, started_at: i64) -> Rec {
    let len = 20 + random.below(400) as usize;

    Rec {
        success: None,
        fail: Some(random.text(len)),
        ..metric(started_at, "billing", "charge", started_at)
    }
}

fn sorted(mut records: Vec<Rec>) -> Vec<Rec> {
    records.sort_by(|a, b| (a.started_at, &a.fail).cmp(&(b.started_at, &b.fail)));
    records
}

/// The records by second, and the first entry of each second, as the chain has them
fn check_chain(file: &HourFile, expected: &BTreeMap<usize, Vec<Rec>>) {
    let entries = file.walk();
    let hour_start = file.hour_start();

    let seconds: Vec<usize> = entries
        .iter()
        .map(|(_, _, time)| second_of(hour_start, *time).unwrap())
        .collect();
    assert!(seconds.windows(2).all(|pair| pair[0] <= pair[1]));

    for second in 0..SECONDS_PER_HOUR {
        let first = entries
            .iter()
            .zip(&seconds)
            .find(|(_, entry_second)| **entry_second == second)
            .map(|((frame, offset, _), _)| Cell {
                frame: *frame,
                offset: *offset as u32,
            });

        assert_eq!(
            first.unwrap_or(Cell::EMPTY),
            file.cells().get(second),
            "second {}",
            second
        );

        assert_eq!(
            file.cells().count(second) as usize,
            expected.get(&second).map_or(0, |records| records.len()),
            "records of second {}",
            second
        );
    }

    let mut by_second: BTreeMap<usize, Vec<Rec>> = BTreeMap::new();
    for record in file.read_all() {
        by_second
            .entry(second_of(hour_start, record.started_at).unwrap())
            .or_default()
            .push(record);
    }

    let by_second: BTreeMap<usize, Vec<Rec>> = by_second
        .into_iter()
        .map(|(second, records)| (second, sorted(records)))
        .collect();

    assert_eq!(&by_second, expected);
}

/// An hour of `seconds` seconds of poorly compressible records, flushed
fn written_hour(dir: &Path, seconds: usize) -> (HourFile, BTreeMap<usize, Vec<Rec>>) {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    let mut file = HourFile::open(dir, HOUR_START, true).unwrap();
    let mut expected = BTreeMap::new();

    for second in 0..seconds {
        let records: Vec<Rec> = (0..30)
            .map(|no| record(&mut random, HOUR_START + second as i64 * MICROS_PER_SECOND + no))
            .collect();

        expected.insert(second, sorted(records.clone()));
        file.write_second(second, &sorted(records), None).unwrap();
    }

    file.flush(true).unwrap();

    (file, expected)
}

#[tokio::test]
async fn seconds_written_in_any_order_are_read_in_time_order() {
    let dir = test_dir("any-order");

    let mut random = Random(0x2545_F491_4F6C_DD1D);
    let mut file = HourFile::open(&dir, HOUR_START, true).unwrap();
    let mut expected: BTreeMap<usize, Vec<Rec>> = BTreeMap::new();

    // Four frames a data file: the chain goes over from file to file all the time
    file.set_data_file_len(4 * FRAME_LEN as u64);

    // The processes the process index keeps seconds of: one which goes on past the second
    // it started in, an id which is not a time, one which started in the hour before.
    // Every other record is a process of its own, over within its second.
    let traced = [
        HOUR_START + 100 * MICROS_PER_SECOND + 7,
        42,
        HOUR_START - MICROS_PER_SECOND,
    ];
    let mut traces: BTreeMap<i64, BTreeSet<usize>> = BTreeMap::new();

    // Seconds come in order, now and then a late one - into a second written before or
    // between two written ones - and the late ones are merged into the last block
    for step in 0..1500 {
        let second = if step % 3 == 0 {
            random.below(600) as usize
        } else {
            (step / 3 + 100).min(SECONDS_PER_HOUR - 1)
        };

        let count = 1 + random.below(40) as usize;
        let records: Vec<Rec> = (0..count)
            .map(|no| {
                let micros = random.below(1_000_000) as i64;
                let mut record =
                    record(&mut random, HOUR_START + second as i64 * MICROS_PER_SECOND + micros);

                if no % 5 == 0 {
                    record.process_id = traced[random.below(3) as usize];
                    traces.entry(record.process_id).or_default().insert(second);
                }

                record
            })
            .collect();

        expected.entry(second).or_default().extend(records.clone());

        let records = sorted(records);

        // Every other second comes compressed, as the storage hands it over
        let blocks = (step % 2 == 0).then(|| compress_records(&records));
        file.write_second(second, &records, blocks).unwrap();

        if step % 100 == 0 {
            file.flush(false).unwrap();
        }
    }

    file.flush(true).unwrap();

    // Every record has its action counted in its second
    for (second, records) in &expected {
        assert_eq!(
            file.whole_key_index().count("billing", "charge", *second),
            Some(records.len() as u32)
        );
    }

    // Every process has its seconds named: the ones it has records in, and the one its
    // id names. That one is not kept - and so nothing is of a process over within it.
    let seconds_of = |process_id: i64| -> Vec<usize> {
        let mut seconds = traces[&process_id].clone();
        seconds.extend(second_of(HOUR_START, process_id));
        seconds.into_iter().collect()
    };

    let process_index = file.whole_process_index();

    for process_id in traced {
        assert_eq!(process_index.seconds_of(process_id), Some(seconds_of(process_id)));
    }

    let kept: usize = traced
        .iter()
        .map(|process_id| {
            traces[process_id]
                .iter()
                .filter(|second| second_of(HOUR_START, *process_id) != Some(**second))
                .count()
        })
        .sum();
    assert_eq!(process_index.pairs(), kept);
    assert_eq!(process_index.total_records(), file.cells().total());

    let expected: BTreeMap<usize, Vec<Rec>> = expected
        .into_iter()
        .map(|(second, records)| (second, sorted(records)))
        .collect();

    check_chain(&file, &expected);
    assert!(file.data_files_count() > 10, "{} data files", file.data_files_count());

    // A scan streams the chain over all the data files
    let mut scanned = Vec::new();
    scan_seconds(file.data_files(), file.cells(), HOUR_START, 0, SECONDS_PER_HOUR - 1, |entry| {
        scanned.push(entry.time);
        true
    })
    .await
    .unwrap();
    let walked: Vec<i64> = file.walk().iter().map(|(_, _, time)| *time).collect();
    assert_eq!(scanned, walked);

    // Read back from the files
    let reopened = HourFile::open(&dir, HOUR_START, true).unwrap();
    assert_eq!(reopened.cells(), file.cells());
    assert_eq!(reopened.whole_key_index(), file.whole_key_index());
    assert_eq!(reopened.whole_process_index(), process_index);

    // An hour which is read keeps only the directories of its key index and process index
    let read_only = HourFile::open(&dir, HOUR_START, false).unwrap();
    assert!(matches!(read_only.key_index(), HourKeyIndex::Directory { .. }));
    assert!(matches!(read_only.process_index(), HourProcessIndex::Directory { .. }));
    for process_id in traced {
        assert_eq!(
            read_only.process_index().seconds_of(process_id).await,
            Some(seconds_of(process_id))
        );
    }
    let counts = read_only.key_index().counts_of("billing", "charge").await;
    for (second, records) in &expected {
        assert_eq!(counts.estimate(u32::MAX, *second), records.len() as u32);
    }

    // The key index is shorter than one read: the read of its page brought in its seconds,
    // and a search reads nothing more - from a file which is not there, it would fail
    let directory = KeyIndexDirectory::read_local(&dir.join(KEY_INDEX_FILE_NAME)).unwrap();
    let nothing_more = HourKeyIndex::Directory {
        directory: Arc::new(directory),
        source: DataSource::Local(dir.join("not-there")),
    };
    let counts = nothing_more.counts_of("billing", "charge").await;
    for (second, records) in &expected {
        assert_eq!(counts.estimate(u32::MAX, *second), records.len() as u32);
    }
    check_chain(&reopened, &expected);

    // An index whose flush did not complete is rebuilt from the data files - and the key
    // index and the process index with it, in the same walk
    let index = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join(SECOND_INDEX_FILE_NAME))
        .unwrap();
    index.write_all_at(&[INDEX_DIRTY], INDEX_STATE_OFFSET).unwrap();
    drop(index);
    std::fs::remove_file(dir.join(KEY_INDEX_FILE_NAME)).unwrap();
    std::fs::remove_file(dir.join(PROCESS_INDEX_FILE_NAME)).unwrap();

    let rebuilt = HourFile::open(&dir, HOUR_START, true).unwrap();
    assert_eq!(rebuilt.cells(), file.cells());
    assert_eq!(rebuilt.whole_key_index(), file.whole_key_index());
    assert_eq!(rebuilt.whole_process_index(), process_index);

    // A key index which does not count what the index does is rebuilt from the data files
    std::fs::remove_file(dir.join(KEY_INDEX_FILE_NAME)).unwrap();
    let rebuilt = HourFile::open(&dir, HOUR_START, false).unwrap();
    assert_eq!(rebuilt.whole_key_index(), file.whole_key_index());

    // And so is a process index: one which is not there, one which is cut
    std::fs::remove_file(dir.join(PROCESS_INDEX_FILE_NAME)).unwrap();
    let rebuilt = HourFile::open(&dir, HOUR_START, false).unwrap();
    assert_eq!(rebuilt.whole_process_index(), process_index);
    drop(rebuilt);

    let process_index_file = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join(PROCESS_INDEX_FILE_NAME))
        .unwrap();
    let len = process_index_file.metadata().unwrap().len();
    process_index_file.set_len(len - 1).unwrap();
    drop(process_index_file);

    let rebuilt = HourFile::open(&dir, HOUR_START, false).unwrap();
    assert_eq!(rebuilt.whole_process_index(), process_index);
    assert_eq!(
        std::fs::metadata(dir.join(PROCESS_INDEX_FILE_NAME)).unwrap().len(),
        len
    );

    // The copy which is uploaded: the same entries, frames one after another - over to
    // the next file where a data file is as long as it grows
    let compact_dir = test_dir("any-order-compact");
    let files = file
        .compact_to(
            |file| compact_dir.join(data_file_name(file)),
            &compact_dir.join(SECOND_INDEX_FILE_NAME),
            &compact_dir.join(KEY_INDEX_FILE_NAME),
            &compact_dir.join(PROCESS_INDEX_FILE_NAME),
        )
        .unwrap();

    let compact = HourFile::open(&compact_dir, HOUR_START, false).unwrap();
    check_chain(&compact, &expected);
    assert_eq!(compact.whole_key_index(), file.whole_key_index());
    assert_eq!(compact.whole_process_index(), process_index);
    assert_eq!(compact.data_files_count(), files as usize);

    let frames: Vec<FrameId> = compact.walk().iter().map(|(frame, _, _)| *frame).collect();
    assert!(frames.windows(2).all(|pair| {
        let next_in_file = FrameId {
            file: pair[0].file,
            offset: pair[0].offset + FRAME_LEN as u32,
        };
        let next_file = FrameId {
            file: pair[0].file + 1,
            offset: 0,
        };
        pair[1] == pair[0] || pair[1] == next_in_file || pair[1] == next_file
    }));

    let size = |dir: &PathBuf| -> u64 {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".data"))
            .map(|entry| entry.metadata().unwrap().len())
            .sum()
    };
    assert!(size(&compact_dir) <= size(&dir));

    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&compact_dir).unwrap();
}

#[test]
fn a_data_file_cut_inside_a_frame_loses_only_that_frame() {
    let dir = test_dir("cut-frame");
    let (file, expected) = written_hour(&dir, 40);
    let cells = file.cells().clone();
    drop(file);

    // A crash in the middle of a write of a new frame: a part of it at the end of the file
    let mut data = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(data_file_name(0)))
        .unwrap();
    data.write_all(&[0xABu8; 1000]).unwrap();
    drop(data);

    let reopened = HourFile::open(&dir, HOUR_START, true).unwrap();

    assert_eq!(reopened.cells(), &cells);
    check_chain(&reopened, &expected);

    // The cut frame is gone, so the next frame does not land behind garbage
    let len = std::fs::metadata(dir.join(data_file_name(0))).unwrap().len();
    assert_eq!(len % FRAME_LEN as u64, 0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_lost_or_short_index_is_rebuilt_from_the_data_files() {
    let dir = test_dir("lost-index");
    let (file, expected) = written_hour(&dir, 40);
    let cells = file.cells().clone();
    let key_index = file.whole_key_index();
    drop(file);

    let index_path = dir.join(SECOND_INDEX_FILE_NAME);

    // The index is derived data: an hour without it is read all the same
    std::fs::remove_file(&index_path).unwrap();

    let reopened = HourFile::open(&dir, HOUR_START, true).unwrap();
    assert_eq!(reopened.cells(), &cells);
    assert_eq!(reopened.whole_key_index(), key_index);
    check_chain(&reopened, &expected);
    drop(reopened);

    assert_eq!(
        std::fs::metadata(&index_path).unwrap().len(),
        INDEX_FILE_LEN as u64
    );

    // A part of it - a write which did not complete
    let index = std::fs::OpenOptions::new()
        .write(true)
        .open(&index_path)
        .unwrap();
    index.set_len(INDEX_FILE_LEN as u64 / 2).unwrap();
    drop(index);

    let reopened = HourFile::open(&dir, HOUR_START, false).unwrap();
    assert_eq!(reopened.cells(), &cells);
    check_chain(&reopened, &expected);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// The hour as it is after a stop which was not clean: the index says it is clean, and
/// the process index - written only when the hour goes quiet - is not there
fn stopped_uncleanly(dir: &Path) -> (u64, HourCells) {
    let (file, expected) = written_hour(dir, 60);
    let cells = file.cells().clone();
    assert!(file.walk().iter().map(|(frame, _, _)| *frame).max().unwrap().offset > 0);
    drop(file);

    std::fs::remove_file(dir.join(PROCESS_INDEX_FILE_NAME)).unwrap();

    let written = expected.values().map(|records| records.len() as u64).sum();
    (written, cells)
}

fn overwrite(path: &Path, offset: u64, bytes: &[u8]) {
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.write_all_at(bytes, offset).unwrap();
}

#[test]
fn an_entry_which_does_not_read_under_a_clean_index_cuts_the_chain_there() {
    let dir = test_dir("torn-entry");
    let (written, _) = stopped_uncleanly(&dir);

    // The first entry of the third frame: its length is not one
    overwrite(
        &dir.join(data_file_name(0)),
        2 * FRAME_LEN as u64 + FRAME_HEADER_LEN as u64,
        &[0xFF; 5],
    );

    // The hour opens with what reads - it is not one which does not open
    let mut reopened = HourFile::open(&dir, HOUR_START, true).unwrap();

    let kept = reopened.cells().total();
    assert!(kept > 0 && kept < written, "{} of {}", kept, written);
    assert_eq!(reopened.read_all().len() as u64, kept);
    assert_eq!(reopened.whole_key_index().total_records(), kept);
    assert_eq!(reopened.whole_process_index().total_records(), kept);

    // And it is written on
    let late = metric(HOUR_START + 3000 * MICROS_PER_SECOND, "billing", "refund", 7);
    reopened.write_second(3000, &[late.clone()], None).unwrap();
    reopened.flush(true).unwrap();
    drop(reopened);

    let again = HourFile::open(&dir, HOUR_START, false).unwrap();
    assert_eq!(again.cells().total(), kept + 1);
    assert_eq!(again.read_all().last(), Some(&late));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_block_which_does_not_read_under_a_clean_index_counts_for_nothing() {
    let dir = test_dir("torn-block");
    let (written, cells) = stopped_uncleanly(&dir);

    // Inside the block of the first entry of the third frame: the entry is whole, what
    // it holds does not decompress
    overwrite(
        &dir.join(data_file_name(0)),
        2 * FRAME_LEN as u64 + FRAME_HEADER_LEN as u64 + 40,
        &[0xFF; 16],
    );

    let reopened = HourFile::open(&dir, HOUR_START, true).unwrap();

    // The chain is whole: only the records of that block are gone - from the counts of
    // the index too, so the indexes agree with it and are not rebuilt with every open
    let kept = reopened.cells().total();
    assert!(kept < written && kept > written - 100, "{} of {}", kept, written);
    assert_eq!(reopened.read_all_which_read().len() as u64, kept);
    assert_eq!(reopened.whole_key_index().total_records(), kept);
    assert_eq!(reopened.whole_process_index().total_records(), kept);

    let last_second = |cells: &HourCells| cells.last().map(|(second, _)| second);
    assert_eq!(last_second(reopened.cells()), last_second(&cells));
    drop(reopened);

    let index_written = |name: &str| std::fs::metadata(dir.join(name)).unwrap().modified().unwrap();
    let before = (index_written(KEY_INDEX_FILE_NAME), index_written(PROCESS_INDEX_FILE_NAME));

    let again = HourFile::open(&dir, HOUR_START, true).unwrap();
    assert_eq!(again.cells().total(), kept);
    assert_eq!(
        (index_written(KEY_INDEX_FILE_NAME), index_written(PROCESS_INDEX_FILE_NAME)),
        before,
        "the indexes agree with the hour: they are not written again"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_record_which_fits_no_entry_is_not_counted() {
    let dir = test_dir("unfit-record");
    let at = |second: i64| HOUR_START + second * MICROS_PER_SECOND;

    // Nothing of it is cut: the client of a record is never dropped, and it names
    // thousands - text which does not compress
    let mut random = Random(0x2545_F491_4F6C_DD1D);
    let mut unfit = metric(at(5) + 1, "billing", "charge", 43);
    unfit.tags = (0..2000)
        .map(|_| EventGrpcTag {
            key: "client_id".to_string(),
            value: random.text(24),
        })
        .collect();

    let mut file = HourFile::open(&dir, HOUR_START, true).unwrap();
    file.write_second(5, &[metric(at(5), "billing", "charge", 42), unfit], None)
        .unwrap();
    file.flush(true).unwrap();

    assert_eq!(file.cells().total(), 1);
    assert_eq!(file.read_all().len(), 1);
    assert_eq!(file.whole_key_index().total_records(), 1);
    assert_eq!(file.whole_process_index().total_records(), 1);
    drop(file);

    // The indexes agree with the hour: none is rebuilt as it opens
    let reopened = HourFile::open(&dir, HOUR_START, false).unwrap();
    assert!(matches!(reopened.key_index(), HourKeyIndex::Directory { .. }));
    assert!(matches!(reopened.process_index(), HourProcessIndex::Directory { .. }));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_hour_whose_tail_did_not_reach_the_disk_opens_and_is_written_on() {
    let dir = test_dir("zeroed-tail");
    let (file, expected) = written_hour(&dir, 60);
    assert!(file.walk().iter().map(|(frame, _, _)| *frame).max().unwrap().offset > 0);
    drop(file);

    // A power loss: the index got to the disk clean, the last frame did not
    let data = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join(data_file_name(0)))
        .unwrap();
    let len = data.metadata().unwrap().len();
    data.write_all_at(&vec![0u8; FRAME_LEN], len - FRAME_LEN as u64)
        .unwrap();
    drop(data);

    let mut reopened = HourFile::open(&dir, HOUR_START, true).unwrap();

    // What was in the lost frame is lost, the seconds before it are all there
    let kept = reopened.cells().total();
    let written: u64 = expected.values().map(|records| records.len() as u64).sum();
    assert!(kept > 0 && kept < written, "{} of {}", kept, written);

    let read: Vec<Rec> = reopened.read_all();
    assert_eq!(read.len() as u64, kept);
    assert_eq!(reopened.whole_key_index().total_records(), kept);
    assert_eq!(reopened.whole_process_index().total_records(), kept);

    // And the hour goes on being written
    let late = metric(HOUR_START + 3000 * MICROS_PER_SECOND, "billing", "refund", 7);
    reopened.write_second(3000, &[late.clone()], None).unwrap();
    reopened.flush(true).unwrap();

    let again = HourFile::open(&dir, HOUR_START, true).unwrap();
    assert_eq!(again.cells().total(), kept + 1);
    assert_eq!(again.read_all().last(), Some(&late));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_process_index_is_not_written_with_every_flush() {
    let dir = test_dir("process-index-flush");
    let at = |second: i64| HOUR_START + second * MICROS_PER_SECOND;

    let mut file = HourFile::open(&dir, HOUR_START, true).unwrap();
    file.write_second(5, &[metric(at(5), "billing", "charge", 42)], None)
        .unwrap();
    file.flush(false).unwrap();

    // The key index goes out with the first flush; the process index waits until the
    // hour has not been written into for a while
    assert!(dir.join(KEY_INDEX_FILE_NAME).is_file());
    assert!(!dir.join(PROCESS_INDEX_FILE_NAME).exists());
    drop(file);

    // As after a crash: it is counted again from the data files, and written
    let mut reopened = HourFile::open(&dir, HOUR_START, true).unwrap();
    assert_eq!(reopened.whole_process_index().seconds_of(42), Some(vec![5]));
    assert!(dir.join(PROCESS_INDEX_FILE_NAME).is_file());

    // The process is about to stop: it is written whatever its time
    reopened
        .write_second(9, &[metric(at(9), "billing", "charge", 42)], None)
        .unwrap();
    reopened.flush(true).unwrap();
    drop(reopened);

    let bytes = std::fs::read(dir.join(PROCESS_INDEX_FILE_NAME)).unwrap();
    let read = ProcessIndex::from_file(HOUR_START, &bytes).unwrap();
    assert_eq!(read.seconds_of(42), Some(vec![5, 9]));
    assert_eq!(read.total_records(), 2);

    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- The storage --------------------------------------------------------------------

use rust_extensions::date_time::DateTimeAsMicroseconds;

fn now_micros() -> i64 {
    DateTimeAsMicroseconds::now().unix_microseconds
}

/// The hour before the current one: it is still written into, and none of its seconds is
/// ahead of the clock - whenever in the hour the test runs
fn writable_hour() -> i64 {
    hour_start_of(now_micros()) - HOUR_MICROS
}

fn scope(name: &str) -> Arc<str> {
    name.into()
}

fn storage_in(dir: &Path) -> Arc<MetricsStorage> {
    Arc::new(MetricsStorage::new(dir, None))
}

/// A storage which uploads its hours into the store, under the prefix `prod-1`
fn storage_with_s3(dir: &Path, store: &Arc<memory::MemoryStore>) -> Arc<MetricsStorage> {
    Arc::new(MetricsStorage::new(
        dir,
        Some(S3Archive::in_store(store.clone(), "prod-1")),
    ))
}

/// A time by which the local time of the hours written now is over
fn three_hours_later() -> DateTimeAsMicroseconds {
    DateTimeAsMicroseconds::new(now_micros() + 3 * HOUR_MICROS)
}

fn hour_folder(dir: &Path, scope: &str, hour_start: i64) -> PathBuf {
    dir.join(METRICS_DIR)
        .join(scope)
        .join(hour_key_of(hour_start).to_string())
}

async fn actions_of(
    storage: &Arc<MetricsStorage>,
    scope_name: &str,
    hour: i64,
    service: &str,
    action: &str,
) -> Vec<Rec> {
    storage
        .search_action(
            scope(scope_name),
            hour,
            service.to_string(),
            action.to_string(),
            None,
            None,
        )
        .await
}

/// The seconds of the hour the events of the process started in, as the storage gives them
async fn seconds_of_process(storage: &Arc<MetricsStorage>, hour: i64, process_id: i64) -> Vec<f64> {
    storage
        .search_process(hour, process_id)
        .await
        .iter()
        .map(|record| (record.started_at - hour) as f64 / MICROS_PER_SECOND as f64)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_are_found_before_and_after_they_are_written() {
    let dir = test_dir("storage-found");
    let storage = storage_in(&dir);
    let hour = writable_hour();
    let at = |second: i64| hour + second * MICROS_PER_SECOND;

    storage.push(
        &scope("default"),
        [
            metric(at(10), "billing", "charge", 1),
            metric(at(20), "billing", "charge", 2),
            metric(at(20) + 5, "billing", "refund", 2),
        ],
    );

    // Nothing is written yet - and a search finds it all the same
    assert_eq!(storage.window_len(), 3);
    assert!(!hour_folder(&dir, "default", hour).exists());

    let check = |charges: Vec<Rec>, process: Vec<Rec>| {
        let started: Vec<i64> = charges.iter().map(|record| record.started_at).collect();
        assert_eq!(started, [at(20), at(10)], "the latest first");

        let actions: Vec<&str> = process
            .iter()
            .map(|record| record.event_data.as_str())
            .collect();
        assert_eq!(actions, ["charge", "refund"], "a trace reads the oldest first");
    };

    check(
        actions_of(&storage, "default", hour, "billing", "charge").await,
        storage.search_process(hour, 2).await,
    );

    storage.flush_all().await;

    assert_eq!(storage.window_len(), 0);

    // An hour is a folder named by its key, in the folder of its scope
    let folder = hour_folder(&dir, "default", hour);
    assert!(folder.join(data_file_name(0)).is_file());
    assert!(folder.join(SECOND_INDEX_FILE_NAME).is_file());
    assert!(folder.join(KEY_INDEX_FILE_NAME).is_file());
    assert!(folder.join(PROCESS_INDEX_FILE_NAME).is_file());

    check(
        actions_of(&storage, "default", hour, "billing", "charge").await,
        storage.search_process(hour, 2).await,
    );

    let hours = storage.hours("default").await;
    assert_eq!(hours.len(), 1);
    assert_eq!(hours[0].0, hour);
    assert!(hours[0].1 > 0);
    assert_eq!(storage.scopes().await, ["default"]);

    // Read back from the files by a storage which has nothing in memory
    let reopened = storage_in(&dir);
    check(
        actions_of(&reopened, "default", hour, "billing", "charge").await,
        reopened.search_process(hour, 2).await,
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_finds_every_metric_while_they_are_written() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let dir = test_dir("storage-search-while-written");
    let storage = storage_in(&dir);
    let hour = writable_hour();

    const PROCESS: i64 = 42;
    const BATCHES: usize = 300;
    const BATCH: usize = 5;

    // How many metrics were handed to the storage: a search which starts then finds them
    let pushed = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));

    let writer = {
        let (storage, pushed, done) = (storage.clone(), pushed.clone(), done.clone());

        tokio::spawn(async move {
            for batch in 0..BATCHES {
                let records: Vec<Rec> = (0..BATCH)
                    .map(|no| {
                        let unique = (batch * BATCH + no) as i64;

                        // Seconds one after another - and every seventh batch into a
                        // second written long before
                        let second = if batch % 7 == 0 { batch % 20 } else { 30 + batch / 3 };

                        metric(
                            hour + second as i64 * MICROS_PER_SECOND + unique,
                            "api",
                            "call",
                            PROCESS,
                        )
                    })
                    .collect();

                storage.push(&scope("default"), records);
                pushed.fetch_add(BATCH, Ordering::SeqCst);

                // The client of a record is named as it is taken: what is written is not
                // the record a search saw waiting, tag for tag
                let ready = storage.take_ready(None, |ready| {
                    for record in ready.iter_mut().flat_map(|bucket| bucket.records.iter_mut()) {
                        record.tags.push(EventGrpcTag {
                            key: "named".to_string(),
                            value: "as it is written".to_string(),
                        });
                    }
                });
                storage.write_ready(ready, false).await;

                tokio::task::yield_now().await;
            }

            done.store(true, Ordering::SeqCst);
        })
    };

    let mut searches = 0;

    while !done.load(Ordering::SeqCst) {
        let pushed_before = pushed.load(Ordering::SeqCst);

        let found = if searches % 2 == 0 {
            storage.search_process(hour, PROCESS).await
        } else {
            // Not the last hundred: the search by action is asked for all of them
            storage
                .action_events_for_test(&scope("default"), hour, "api", "call", BATCHES * BATCH)
                .await
        };

        let times: BTreeSet<i64> = found.iter().map(|record| record.started_at).collect();

        assert_eq!(times.len(), found.len(), "a metric is found twice");
        assert!(
            found.len() >= pushed_before,
            "{} metrics were handed over before the search, and it found {}",
            pushed_before,
            found.len()
        );

        searches += 1;
    }

    writer.await.unwrap();

    assert!(searches > 20, "{} searches ran while the metrics were written", searches);
    assert_eq!(storage.search_process(hour, PROCESS).await.len(), BATCHES * BATCH);
    assert_eq!(storage.window_len(), 0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn late_metrics_fall_into_their_place() {
    let dir = test_dir("storage-late");
    let storage = storage_in(&dir);
    let hour = writable_hour();
    let at = |second: i64| hour + second * MICROS_PER_SECOND;

    // Out of order within what waits in memory: written in order
    storage.push(
        &scope("default"),
        [
            metric(at(30), "billing", "c", 1),
            metric(at(10), "billing", "a", 1),
            metric(at(20), "billing", "b", 1),
        ],
    );
    storage.flush_all().await;

    assert_eq!(seconds_of_process(&storage, hour, 1).await, [10.0, 20.0, 30.0]);

    // Older than everything written: between two written seconds, and into a written one
    storage.push(
        &scope("default"),
        [
            metric(at(15), "billing", "late", 1),
            metric(at(10) + 500_000, "billing", "same-second", 1),
        ],
    );
    storage.flush_all().await;

    assert_eq!(
        seconds_of_process(&storage, hour, 1).await,
        [10.0, 10.5, 15.0, 20.0, 30.0]
    );

    // And it is so in the files, not only in what the storage keeps of them
    let reopened = storage_in(&dir);
    assert_eq!(
        seconds_of_process(&reopened, hour, 1).await,
        [10.0, 10.5, 15.0, 20.0, 30.0]
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_events_of_an_action_are_the_latest_and_filtered() {
    let dir = test_dir("storage-last-events");
    let storage = storage_in(&dir);
    let hour = writable_hour();
    let at = |second: i64| hour + second * MICROS_PER_SECOND;

    let mut other_client = metric(at(149) + 500_000, "my-service", "GET /orders", 5000);
    other_client.tags[0].value = "client-2".to_string();

    let records = (0..150i64)
        .map(|second| metric(at(second), "my-service", "GET /orders", second))
        .chain([metric(at(200), "my-service", "GET /users", 999), other_client]);

    storage.push(&scope("default"), records);
    storage.flush_all().await;

    let found = actions_of(&storage, "default", hour, "my-service", "GET /orders").await;

    // The latest first, as many as the screen takes, and the other action is not among them
    assert_eq!(found.len(), LAST_EVENTS_LIMIT);
    assert_eq!(found[0].started_at, at(149) + 500_000);
    assert_eq!(found[1].started_at, at(149));
    assert_eq!(found[LAST_EVENTS_LIMIT - 1].started_at, at(51));
    assert!(found.iter().all(|record| record.event_data == "GET /orders"));

    let search = |client_id: Option<&str>, from: Option<i64>| {
        storage.search_action(
            scope("default"),
            hour,
            "my-service".to_string(),
            "GET /orders".to_string(),
            client_id.map(str::to_string),
            from,
        )
    };

    let of_client = search(Some("client-2"), None).await;
    assert_eq!(of_client.len(), 1);
    assert_eq!(of_client[0].process_id, 5000);

    // From a second on: the seconds before it are not given
    let from_140 = search(None, Some(at(140))).await;
    assert_eq!(from_140.len(), 11);
    assert!(from_140.iter().all(|record| record.started_at >= at(140)));

    // The same name of an action in a service which has none
    assert!(actions_of(&storage, "default", hour, "other-service", "GET /orders")
        .await
        .is_empty());

    // An hour which has no metrics
    assert!(actions_of(&storage, "default", hour - HOUR_MICROS, "my-service", "GET /orders")
        .await
        .is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn scopes_are_kept_apart_and_a_process_is_found_in_all_of_them() {
    let dir = test_dir("storage-scopes");
    let storage = storage_in(&dir);
    let hour = writable_hour();
    let at = |second: i64| hour + second * MICROS_PER_SECOND;

    storage.push(&scope("default"), [metric(at(5), "api", "call", 77)]);
    storage.push(
        &scope("trading"),
        [
            metric(at(6), "engine", "match", 77),
            metric(at(7), "api", "call", 78),
        ],
    );

    // Waiting in memory, the scopes are apart already
    assert_eq!(storage.scopes().await, ["default", "trading"]);
    assert_eq!(actions_of(&storage, "trading", hour, "api", "call").await.len(), 1);

    storage.flush_all().await;

    assert!(hour_folder(&dir, "default", hour).is_dir());
    assert!(hour_folder(&dir, "trading", hour).is_dir());
    assert_eq!(storage.scopes().await, ["default", "trading"]);
    assert_eq!(storage.hours("trading").await.len(), 1);
    assert!(storage.hours("prop").await.is_empty());
    // Not a scope: it would be read as a path
    assert!(storage.hours("../metrics/default").await.is_empty());

    // The same service and action in two scopes are two different ones
    let in_default = actions_of(&storage, "default", hour, "api", "call").await;
    assert_eq!(in_default.len(), 1);
    assert_eq!(in_default[0].process_id, 77);

    let in_trading = actions_of(&storage, "trading", hour, "api", "call").await;
    assert_eq!(in_trading.len(), 1);
    assert_eq!(in_trading[0].process_id, 78);

    // A process goes through the services of both: its trace is whole
    let trace = storage.search_process(hour, 77).await;
    let services: Vec<&str> = trace
        .iter()
        .map(|record| record.service_name.as_str())
        .collect();
    assert_eq!(services, ["api", "engine"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// The events of two processes in the hour: one whose id is the time it started - with an
/// event of a service whose clock is a little behind - and one whose id is not a time, as
/// a writer over HTTP may give
fn two_processes(hour: i64) -> (i64, i64, Vec<Rec>) {
    let at = |second: i64| hour + second * MICROS_PER_SECOND;

    let process_id = at(1000);
    let not_a_time = 42;

    let records = vec![
        metric(at(100), "api", "old", not_a_time),
        metric(at(1000) - 2 * MICROS_PER_SECOND, "session", "get", process_id),
        metric(at(1000), "api", "call", process_id),
        metric(at(1300), "billing", "charge", process_id),
        metric(at(3000), "api", "old", not_a_time),
        // The other processes of the hour
        metric(at(999), "api", "call", at(999)),
        metric(at(1300), "billing", "charge", at(1300) - 10),
    ];

    (process_id, not_a_time, records)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trace_reads_the_seconds_the_process_index_names() {
    let dir = test_dir("storage-trace");
    let storage = storage_in(&dir);
    let hour = writable_hour();

    let (process_id, not_a_time, records) = two_processes(hour);

    storage.push(&scope("default"), records);
    storage.flush_all().await;

    assert_eq!(
        seconds_of_process(&storage, hour, process_id).await,
        [998.0, 1000.0, 1300.0]
    );
    assert_eq!(seconds_of_process(&storage, hour, not_a_time).await, [100.0, 3000.0]);

    // A process the hour has nothing of
    assert!(seconds_of_process(&storage, hour, hour + 5).await.is_empty());
    assert!(seconds_of_process(&storage, hour, 43).await.is_empty());
    assert!(seconds_of_process(&storage, hour, i64::MIN).await.is_empty());

    // Events which come later add their seconds
    storage.push(
        &scope("default"),
        [metric(hour + 2000 * MICROS_PER_SECOND, "billing", "refund", process_id)],
    );
    storage.flush_all().await;

    assert_eq!(
        seconds_of_process(&storage, hour, process_id).await,
        [998.0, 1000.0, 1300.0, 2000.0]
    );

    // And it is so in the files
    let reopened = storage_in(&dir);
    assert_eq!(
        seconds_of_process(&reopened, hour, process_id).await,
        [998.0, 1000.0, 1300.0, 2000.0]
    );
    assert_eq!(seconds_of_process(&reopened, hour, not_a_time).await, [100.0, 3000.0]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trace_scans_the_hour_when_its_process_index_says_nothing() {
    let dir = test_dir("storage-trace-scan");

    // Two hours back: still written into, and read with only the directories of its
    // indexes in memory
    let hour = hour_start_of(now_micros()) - 2 * HOUR_MICROS;

    let (process_id, not_a_time, records) = two_processes(hour);

    let writer = storage_in(&dir);
    writer.push(&scope("default"), records);
    writer.flush_all().await;
    drop(writer);

    let storage = storage_in(&dir);

    assert_eq!(
        seconds_of_process(&storage, hour, process_id).await,
        [998.0, 1000.0, 1300.0]
    );

    // The file of the process index is cut after the hour was opened: its pages do not
    // read any more
    let path = hour_folder(&dir, "default", hour).join(PROCESS_INDEX_FILE_NAME);
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let len = file.metadata().unwrap().len();
    file.set_len(len - 1).unwrap();
    drop(file);

    // The hour is scanned from the second the process started in - a minute before it,
    // for the clocks which are behind - and whole for an id which is not a time
    assert_eq!(
        seconds_of_process(&storage, hour, process_id).await,
        [998.0, 1000.0, 1300.0]
    );
    assert_eq!(seconds_of_process(&storage, hour, not_a_time).await, [100.0, 3000.0]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_of_an_hour_which_is_not_written_into_any_more_are_dropped() {
    let dir = test_dir("storage-too-late");
    let storage = storage_in(&dir);
    let old_hour = hour_start_of(now_micros()) - 4 * HOUR_MICROS;

    storage.push(
        &scope("default"),
        [metric(old_hour + MICROS_PER_SECOND, "billing", "charge", 1)],
    );

    let writer = storage.clone();
    let messages = tokio::task::spawn_blocking(move || {
        let ready = vec![Arc::new(ReadyBucket {
            scope: scope("default"),
            second: (old_hour + MICROS_PER_SECOND) / MICROS_PER_SECOND,
            records: vec![metric(old_hour + MICROS_PER_SECOND, "billing", "charge", 1)],
        })];

        writer.write(ready)
    })
    .await
    .unwrap();

    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("are dropped"), "{}", messages[0]);
    assert!(!hour_folder(&dir, "default", old_hour).exists());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn gc_deletes_the_hours_whose_local_time_is_over() {
    let dir = test_dir("storage-gc");
    let storage = storage_in(&dir);
    let hour = writable_hour();
    let old_hour = hour_start_of(now_micros()) - 4 * HOUR_MICROS;

    storage.push(&scope("default"), [metric(hour + 5, "billing", "charge", 1)]);
    storage.flush_all().await;

    // An hour written when it was the current one
    let old_folder = hour_folder(&dir, "default", old_hour);
    std::fs::create_dir_all(&old_folder).unwrap();
    let mut old = HourFile::open(&old_folder, old_hour, true).unwrap();
    old.write_second(7, &[metric(old_hour + 7 * MICROS_PER_SECOND, "billing", "charge", 2)], None)
        .unwrap();
    old.flush(true).unwrap();
    drop(old);

    // It is read while it is there
    assert_eq!(storage.hours("default").await.len(), 2);
    assert_eq!(actions_of(&storage, "default", old_hour, "billing", "charge").await.len(), 1);
    assert_eq!(storage.open_hours(), 2);

    let collector = storage.clone();
    let messages =
        tokio::task::spawn_blocking(move || collector.gc(DateTimeAsMicroseconds::now(), 168))
            .await
            .unwrap();

    assert_eq!(
        messages,
        [format!("GC: default/{} is deleted", hour_key_of(old_hour))]
    );
    assert!(!old_folder.exists());
    assert!(hour_folder(&dir, "default", hour).is_dir());

    assert_eq!(storage.hours("default").await.len(), 1);
    assert_eq!(storage.open_hours(), 1);
    assert!(actions_of(&storage, "default", old_hour, "billing", "charge")
        .await
        .is_empty());
    assert_eq!(actions_of(&storage, "default", hour, "billing", "charge").await.len(), 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- S3 -----------------------------------------------------------------------------

/// An hour of 300 seconds of bulky records - megabytes of frames - with a process which
/// goes on from second 100 into the next one, and an action of its own in second 200
fn bulky_hour(hour: i64) -> (i64, Vec<Rec>) {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    let at = |second: i64| hour + second * MICROS_PER_SECOND;

    let process_id = at(100) + 500_000;

    let mut records = Vec::new();

    for second in 0..300 {
        for no in 0..30 {
            records.push(record(&mut random, at(second) + no));
        }
    }

    records.push(metric(process_id, "api", "call", process_id));
    records.push(metric(at(101) + 7, "engine", "match", process_id));
    records.push(metric(at(200), "api", "rare", at(200)));

    (process_id, records)
}

fn keys_of(store: &memory::MemoryStore, scope: &str, hour: i64) -> Vec<String> {
    let prefix = format!("prod-1/{}/{}/", scope, hour_key_of(hour));

    store
        .keys()
        .into_iter()
        .filter_map(|key| key.strip_prefix(&prefix).map(str::to_string))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_hour_is_uploaded_and_read_from_s3() {
    let dir = test_dir("s3-upload");
    let store = Arc::new(memory::MemoryStore::default());
    let storage = storage_with_s3(&dir, &store);
    let hour = writable_hour();

    let (process_id, records) = bulky_hour(hour);
    storage.push(&scope("default"), records);
    storage.push(&scope("trading"), [metric(hour + 5, "engine", "quote", 7)]);
    storage.flush_all().await;

    // While it is written into, an hour stays where it is
    let now = DateTimeAsMicroseconds::now();
    assert!(storage.hours_to_upload(now, 168).is_empty());

    // Two hours after it ended it is due - unless it is not kept that long
    let due = storage.hours_to_upload(three_hours_later(), 168);
    assert_eq!(due, [(scope("default"), hour), (scope("trading"), hour)]);
    assert!(storage.hours_to_upload(three_hours_later(), 3).is_empty());

    let trace_before = storage.search_process(hour, process_id).await;
    let rare_before = actions_of(&storage, "default", hour, "api", "rare").await;
    assert_eq!(trace_before.len(), 2);
    assert_eq!(rare_before.len(), 1);

    for (scope, hour_start) in due {
        let message = storage.upload_hour(scope.clone(), hour_start).await.unwrap();
        assert_eq!(message, format!("S3: {}/{} is uploaded", scope, hour_key_of(hour)));
    }

    // The folders are gone, with the copies which were uploaded; the hours are in S3,
    // their files named as they were
    assert!(!hour_folder(&dir, "default", hour).exists());
    assert!(!hour_folder(&dir, "trading", hour).exists());
    assert_eq!(storage.open_hours(), 0);

    let files = ["key.index", "metrics.data00", "process.index", "second.index"];
    assert_eq!(keys_of(&store, "default", hour), files);
    assert_eq!(keys_of(&store, "trading", hour), files);

    assert!(storage.hours_to_upload(three_hours_later(), 168).is_empty());

    // They are listed as they were - by what S3 has now
    assert_eq!(storage.scopes().await, ["default", "trading"]);

    let hours = storage.hours("default").await;
    assert_eq!(hours.len(), 1);
    assert_eq!(hours[0].0, hour);

    let data_len = store.objects.lock()[&format!(
        "prod-1/default/{}/metrics.data00",
        hour_key_of(hour)
    )]
        .len() as u64;
    assert!(data_len > 1024 * 1024, "{} bytes of data", data_len);
    assert!(hours[0].1 > data_len);

    // And read as they were: a trace by the process index - the two seconds it names, not
    // the hour - and the events of an action by the key index
    store.bytes_asked.lock().clear();

    assert_eq!(storage.search_process(hour, process_id).await, trace_before);

    let data_asked = |store: &memory::MemoryStore| -> u64 {
        store
            .bytes_asked
            .lock()
            .iter()
            .filter(|(key, _)| key.contains("metrics.data"))
            .map(|(_, bytes)| *bytes)
            .sum()
    };

    let trace_asked = data_asked(&store);
    assert!(
        trace_asked > 0 && trace_asked < data_len / 5,
        "{} of {} bytes are read for a trace",
        trace_asked,
        data_len
    );

    store.bytes_asked.lock().clear();
    assert_eq!(actions_of(&storage, "default", hour, "api", "rare").await, rare_before);
    assert!(data_asked(&store) < data_len / 5);

    assert_eq!(
        actions_of(&storage, "trading", hour, "engine", "quote").await.len(),
        1
    );

    // Another server process reads them the same - from S3 alone
    let other_dir = test_dir("s3-upload-other");
    let other = storage_with_s3(&other_dir, &store);
    assert_eq!(other.search_process(hour, process_id).await, trace_before);
    assert_eq!(other.hours("trading").await.len(), 1);

    // An hour whose process index is not in S3: its traces scan it
    let process_index_key = format!("prod-1/default/{}/process.index", hour_key_of(hour));
    let process_index = store.objects.lock().remove(&process_index_key).unwrap();

    let scanning = storage_with_s3(&other_dir, &store);

    store.bytes_asked.lock().clear();
    assert_eq!(scanning.search_process(hour, process_id).await, trace_before);
    assert!(data_asked(&store) > data_len / 5);

    // It is there again: the same storage reads the trace by it, not by a scan it would
    // have gone on with for as long as it kept what it was given without the index
    store.objects.lock().insert(process_index_key, process_index);

    store.bytes_asked.lock().clear();
    assert_eq!(scanning.search_process(hour, process_id).await, trace_before);
    assert!(data_asked(&store) < data_len / 5);

    // What is not kept any more is deleted from S3: every file of the hour
    let kept = DateTimeAsMicroseconds::new(hour + 100 * HOUR_MICROS);
    assert!(storage.gc_s3(kept, 168).await.unwrap().is_empty());

    let expired = DateTimeAsMicroseconds::new(hour + 169 * HOUR_MICROS);
    assert_eq!(
        storage.gc_s3(expired, 168).await.unwrap(),
        [
            format!("default/{}", hour_key_of(hour)),
            format!("trading/{}", hour_key_of(hour))
        ]
    );

    assert!(store.keys().is_empty());
    assert!(storage.hours("default").await.is_empty());
    assert_eq!(storage.scopes().await, ["default"]);
    assert!(storage.search_process(hour, process_id).await.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&other_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_which_fails_is_tried_again_later() {
    let dir = test_dir("s3-upload-fails");
    let store = Arc::new(memory::MemoryStore::default());
    let storage = storage_with_s3(&dir, &store);
    let hour = writable_hour();

    storage.push(&scope("default"), [metric(hour + 5, "billing", "charge", 1)]);
    storage.flush_all().await;

    store.is_down.store(true, std::sync::atomic::Ordering::Relaxed);

    let err = storage.upload_hour(scope("default"), hour).await.unwrap_err();
    assert!(err.contains("The store is down"), "{}", err);

    // The hour is where it was, without the copies made for the upload, and it is read
    let folder = hour_folder(&dir, "default", hour);
    let mut files: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    files.sort();
    assert_eq!(files, ["key.index", "metrics.data00", "process.index", "second.index"]);
    assert!(store.keys().is_empty());
    assert_eq!(actions_of(&storage, "default", hour, "billing", "charge").await.len(), 1);

    // It is not tried again at once - a minute later it is, after the second failure two
    // minutes later
    let later = three_hours_later();
    assert!(storage.hours_to_upload(later, 168).is_empty());
    assert_eq!(
        storage.hours_to_upload_after(later, 168, Duration::from_secs(61)),
        [(scope("default"), hour)]
    );

    storage.upload_hour(scope("default"), hour).await.unwrap_err();
    assert!(storage
        .hours_to_upload_after(later, 168, Duration::from_secs(61))
        .is_empty());
    assert_eq!(
        storage.hours_to_upload_after(later, 168, Duration::from_secs(121)).len(),
        1
    );

    // S3 is back - and takes everything but the index, which is uploaded the last: the
    // hour is not in S3 yet, and it is still read from its folder
    store.is_down.store(false, std::sync::atomic::Ordering::Relaxed);
    *store.refuses.lock() = Some("second.index".to_string());

    storage.upload_hour(scope("default"), hour).await.unwrap_err();
    assert_eq!(
        keys_of(&store, "default", hour),
        ["key.index", "metrics.data00", "process.index"]
    );
    assert!(storage.hours("default").await.iter().all(|(_, size)| *size > 0));
    assert!(folder.join(data_file_name(0)).is_file());
    assert_eq!(actions_of(&storage, "default", hour, "billing", "charge").await.len(), 1);

    // The next try goes on from where this one stopped: the data file is there already
    *store.refuses.lock() = None;

    storage.upload_hour(scope("default"), hour).await.unwrap();
    assert!(!folder.exists());
    assert_eq!(keys_of(&store, "default", hour).len(), 4);
    assert_eq!(actions_of(&storage, "default", hour, "billing", "charge").await.len(), 1);

    let data_uploads = store
        .uploaded
        .lock()
        .iter()
        .filter(|key| key.ends_with("metrics.data00"))
        .count();
    assert_eq!(data_uploads, 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_hour_which_is_in_s3_already_is_not_uploaded_again() {
    let dir = test_dir("s3-already-there");
    let store = Arc::new(memory::MemoryStore::default());
    let hour = writable_hour();

    // Uploaded by a process which stopped before it deleted the folder
    let writer = storage_in(&dir);
    writer.push(&scope("default"), [metric(hour + 5, "billing", "charge", 1)]);
    writer.flush_all().await;
    drop(writer);

    let index_key = format!("prod-1/default/{}/second.index", hour_key_of(hour));
    store.objects.lock().insert(index_key.clone(), vec![1, 2, 3]);

    let storage = storage_with_s3(&dir, &store);

    let message = storage.upload_hour(scope("default"), hour).await.unwrap();
    assert!(message.contains("is there already"), "{}", message);

    assert!(!hour_folder(&dir, "default", hour).exists());
    assert_eq!(store.keys(), [index_key]);

    std::fs::remove_dir_all(&dir).unwrap();
}
