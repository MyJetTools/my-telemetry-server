use std::sync::Arc;

use super::{
    decompress_records, scan_seconds, DataFiles, Entry, FilterCounts, HourCells, MetricsQuery,
    Rec, DATA_FILE_LEN, FRAME_LEN,
};

/// What one read of a local data file brings in: a window is not read for less
const LOCAL_READ_LEN: u64 = 64 * 1024;

/// What a request to S3 is worth: one costs as much as reading this much more
const S3_READ_LEN: u64 = 1024 * 1024;

/// The data files of an hour as a search reads them, with what its indexes say of the
/// search. They are what the hour was when it was opened for the pass: a pass reads them
/// under the read lock of the hour, the next one opens the hour anew.
pub struct HourSource {
    pub hour_start: i64,
    pub data: DataFiles,
    pub cells: Arc<HourCells>,
    pub counts: FilterCounts,
}

impl HourSource {
    /// The records of the second
    pub fn count(&self, second: usize) -> u64 {
        self.cells.count(second) as u64
    }

    /// At most how many records of the second match
    pub fn estimate(&self, second: usize) -> u64 {
        self.counts.estimate(self.cells.count(second), second) as u64
    }

    /// The seconds of `from_second ..= to_second` to read, as runs read in one pass each:
    /// the seconds which may hold a match, and between two of them the seconds which do
    /// not, while they hold fewer records than one read brings in
    pub fn runs(
        &self,
        from_second: usize,
        to_second: usize,
        records_per_read: u64,
    ) -> Vec<(usize, usize)> {
        let mut result = Vec::new();
        let mut current: Option<(usize, usize)> = None;
        let mut gap = 0;

        for second in from_second..=to_second {
            let records = self.count(second);

            if records == 0 {
                continue;
            }

            if self.estimate(second) == 0 {
                gap += records;
                continue;
            }

            current = match current {
                Some((start, _)) if gap < records_per_read => Some((start, second)),
                Some(run) => {
                    result.push(run);
                    Some((second, second))
                }
                None => Some((second, second)),
            };

            gap = 0;
        }

        result.extend(current);
        result
    }

    /// About how many records one read brings in: the records of the hour by the bytes of
    /// its data files, the index alone telling both
    pub fn records_per_read(&self) -> u64 {
        let Some((_, last)) = self.cells.last() else {
            return 0;
        };

        let bytes =
            last.frame.file as u64 * DATA_FILE_LEN + last.frame.offset as u64 + FRAME_LEN as u64;

        let read_len = match self.data {
            DataFiles::Local { .. } => LOCAL_READ_LEN,
            DataFiles::S3 { .. } => S3_READ_LEN,
        };

        read_len * self.cells.total() / bytes
    }

    /// Reads the seconds `from_second ..= to_second` and gives `out` the records of them in
    /// `from ..= to` which match
    pub async fn collect(
        &self,
        from_second: usize,
        to_second: usize,
        from: i64,
        to: i64,
        query: &MetricsQuery<'_>,
        out: &mut Vec<Rec>,
    ) {
        let scanned = scan_seconds(
            self.data.clone(),
            &self.cells,
            self.hour_start,
            from_second,
            to_second,
            |entry| {
                collect_matches(entry, from, to, query, out);
                true
            },
        )
        .await;

        if let Err(err) = scanned {
            println!(
                "The hour {} is not read to its end: {}",
                super::hour_key_of(self.hour_start),
                err
            );
        }
    }
}

/// The records of the entry within `from ..= to` which match
fn collect_matches(entry: &Entry, from: i64, to: i64, query: &MetricsQuery<'_>, out: &mut Vec<Rec>) {
    match decompress_records(entry.payload) {
        Ok(records) => {
            for record in records {
                if record.started_at >= from && record.started_at <= to && query.matches(&record) {
                    out.push(record);
                }
            }
        }
        Err(err) => println!("A block does not read: {}", err),
    }
}

/// How many records which may match the next pass is to take in: what is missing, scaled
/// by the share of the estimated ones which matched in the pass before
pub fn next_wanted(missing: u64, estimated: u64, matched: u64) -> u64 {
    if matched == 0 {
        estimated.max(1) * 2
    } else {
        (missing * estimated).div_ceil(matched)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::metrics_storage::{tests::metric, Cell, FrameId, HourKeyIndex, KeyIndex};

    #[tokio::test]
    async fn seconds_far_apart_are_read_in_passes_of_their_own() {
        let mut cells = HourCells::new();
        for second in 0..1000 {
            cells.set(
                second,
                Cell {
                    frame: FrameId::HEAD,
                    offset: 11,
                },
            );
            cells.set_count(second, 100);
        }

        let mut key_index = KeyIndex::new();
        for second in 0..1000 {
            key_index.add_records(second, &[metric(0, "billing", "frequent", 1)]);
        }
        for second in [10, 11, 500] {
            key_index.add_records(second, &[metric(0, "billing", "rare", 1)]);
        }

        let key_index = HourKeyIndex::Full(Arc::new(key_index));
        let cells = Arc::new(cells);

        let source_of = |counts: FilterCounts| HourSource {
            hour_start: 0,
            data: DataFiles::Local {
                dir: PathBuf::new(),
            },
            cells: cells.clone(),
            counts,
        };

        let source = source_of(key_index.counts_of("billing", "rare").await);

        // 488 seconds of 100 records between them: more than one read brings in
        assert_eq!(source.runs(0, 999, 1000), [(10, 11), (500, 500)]);

        // Fewer than one read: read through
        assert_eq!(source.runs(0, 999, 100_000), [(10, 500)]);

        // An action of another service of the same name is not in the hour at all
        let source = source_of(key_index.counts_of("trading", "rare").await);
        assert!(source.runs(0, 999, 1000).is_empty());

        // A search the index says nothing of: every second may match
        let source = source_of(FilterCounts::unknown());
        assert_eq!(source.runs(0, 999, 1000), [(0, 999)]);
    }
}
