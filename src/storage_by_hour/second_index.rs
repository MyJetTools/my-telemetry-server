pub const SECONDS_WITHIN_HOUR: usize = 3600;

/// One slot is a u32 offset into the data file. That caps an hour file at 4Gb, which is
/// well past the ~1Gb an hour is expected to weigh, and halves the index against a u64.
pub const SLOT_SIZE: usize = std::mem::size_of::<u32>();

/// The whole index is 14.4Kb - small enough to keep in memory for every open hour and to
/// rewrite wholesale if we ever need to.
pub const INDEX_FILE_LEN: usize = SECONDS_WITHIN_HOUR * SLOT_SIZE;

/// Largest offset a slot can address.
pub const MAX_DATA_FILE_LEN: u64 = (u32::MAX - 1) as u64;

/// A second nothing was ever written to. Distinct from offset 0, which is a real position.
const EMPTY_SLOT: u32 = u32::MAX;

/// Second of the hour a metric belongs to, from its absolute unix-microsecond timestamp.
pub fn second_within_hour(started_micros: i64) -> usize {
    let unix_seconds = started_micros.div_euclid(1_000_000);
    unix_seconds.rem_euclid(SECONDS_WITHIN_HOUR as i64) as usize
}

/// Maps each second of the hour to the offset in the data file where that second starts.
///
/// Lives in memory for as long as the hour is open, and is flushed next to the data file.
/// It is fully rebuildable by scanning the data file, so a lost or torn index is a
/// recoverable condition, not data loss.
pub struct SecondIndex {
    slots: Vec<u32>,
}

impl SecondIndex {
    pub fn new() -> Self {
        Self {
            slots: vec![EMPTY_SLOT; SECONDS_WITHIN_HOUR],
        }
    }

    /// `None` if the buffer is not a whole index - it gets rebuilt from the data file.
    pub fn from_bytes(src: &[u8]) -> Option<Self> {
        if src.len() != INDEX_FILE_LEN {
            return None;
        }

        let mut slots = Vec::with_capacity(SECONDS_WITHIN_HOUR);

        for chunk in src.chunks_exact(SLOT_SIZE) {
            slots.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }

        Some(Self { slots })
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(INDEX_FILE_LEN);

        for slot in self.slots.iter() {
            result.extend_from_slice(&slot.to_le_bytes());
        }

        result
    }

    /// Where to start reading to see every record of `second` and everything after it.
    ///
    /// Empty seconds are skipped forward, so this answers "the first record at or after
    /// this second" - which is also exactly where a record for an empty `second` belongs.
    /// `None` means nothing on disk is that late: the caller appends at EOF.
    pub fn offset_at_or_after(&self, second: usize) -> Option<u64> {
        self.slots
            .get(second..)?
            .iter()
            .find(|slot| **slot != EMPTY_SLOT)
            .map(|slot| *slot as u64)
    }

    /// Records the start of a second. The first record written into a second wins - later
    /// ones are further along and must not move the start backwards or forwards.
    pub fn set_start_of_second(&mut self, second: usize, offset: u64) {
        if second >= SECONDS_WITHIN_HOUR {
            return;
        }

        if self.slots[second] == EMPTY_SLOT {
            self.slots[second] = offset as u32;
        }
    }

    /// Everything strictly after `second` moved by `delta` bytes because a record was
    /// inserted into `second`. The delta is the size of that record - constant for every
    /// later slot, so this is an add, never a recompute.
    pub fn shift_after(&mut self, second: usize, delta: u64) {
        let from = second + 1;

        if from >= SECONDS_WITHIN_HOUR {
            return;
        }

        for slot in self.slots[from..].iter_mut() {
            if *slot != EMPTY_SLOT {
                *slot += delta as u32;
            }
        }
    }

    /// Latest second that holds data. This is what decides whether an incoming record is
    /// an append or a heavy insert.
    pub fn last_non_empty_second(&self) -> Option<usize> {
        self.slots.iter().rposition(|slot| *slot != EMPTY_SLOT)
    }

    /// Drops everything - used when the index is rebuilt from the data file.
    pub fn reset(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = EMPTY_SLOT;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_within_hour_is_relative_to_the_utc_hour() {
        // 1970-01-01 00:00:00 UTC
        assert_eq!(second_within_hour(0), 0);
        // 90 seconds past the hour
        assert_eq!(second_within_hour(90 * 1_000_000), 90);
        // last second of the hour
        assert_eq!(second_within_hour(3599 * 1_000_000), 3599);
        // first second of the next hour wraps back to 0
        assert_eq!(second_within_hour(3600 * 1_000_000), 0);
        // sub second precision does not leak into the second
        assert_eq!(second_within_hour(90 * 1_000_000 + 999_999), 90);
    }

    #[test]
    fn first_write_into_a_second_wins() {
        let mut index = SecondIndex::new();

        index.set_start_of_second(10, 500);
        index.set_start_of_second(10, 900);

        assert_eq!(index.offset_at_or_after(10), Some(500));
    }

    #[test]
    fn empty_seconds_are_skipped_forward() {
        let mut index = SecondIndex::new();

        index.set_start_of_second(10, 500);
        index.set_start_of_second(20, 900);

        // second 15 holds nothing - the next data at or after it starts at second 20
        assert_eq!(index.offset_at_or_after(15), Some(900));
        assert_eq!(index.offset_at_or_after(10), Some(500));
        // nothing on disk is that late
        assert_eq!(index.offset_at_or_after(21), None);
    }

    #[test]
    fn shift_after_moves_only_later_seconds() {
        let mut index = SecondIndex::new();

        index.set_start_of_second(10, 100);
        index.set_start_of_second(20, 200);
        index.set_start_of_second(30, 300);

        index.shift_after(20, 50);

        assert_eq!(index.offset_at_or_after(10), Some(100));
        assert_eq!(index.offset_at_or_after(20), Some(200));
        assert_eq!(index.offset_at_or_after(30), Some(350));
    }

    #[test]
    fn shift_after_leaves_empty_slots_empty() {
        let mut index = SecondIndex::new();

        index.set_start_of_second(30, 300);
        index.shift_after(10, 50);

        assert_eq!(index.offset_at_or_after(20), Some(350));
        assert_eq!(index.offset_at_or_after(31), None);
    }

    #[test]
    fn bytes_round_trip() {
        let mut index = SecondIndex::new();
        index.set_start_of_second(0, 0);
        index.set_start_of_second(3599, 123_456);

        let bytes = index.as_bytes();
        assert_eq!(bytes.len(), INDEX_FILE_LEN);

        let back = SecondIndex::from_bytes(&bytes).unwrap();

        assert_eq!(back.offset_at_or_after(0), Some(0));
        assert_eq!(back.offset_at_or_after(3599), Some(123_456));
        assert_eq!(back.offset_at_or_after(1), Some(123_456));
    }

    #[test]
    fn a_short_index_file_is_rejected_so_it_can_be_rebuilt() {
        assert!(SecondIndex::from_bytes(&[0u8; 16]).is_none());
        assert!(SecondIndex::from_bytes(&[]).is_none());
    }

    #[test]
    fn last_non_empty_second_decides_append_vs_insert() {
        let mut index = SecondIndex::new();
        assert_eq!(index.last_non_empty_second(), None);

        index.set_start_of_second(10, 100);
        index.set_start_of_second(30, 300);
        index.set_start_of_second(20, 200);

        assert_eq!(index.last_non_empty_second(), Some(30));
    }
}
