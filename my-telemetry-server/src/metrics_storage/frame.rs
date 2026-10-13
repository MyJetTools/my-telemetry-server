use rust_extensions::{ParseUInt32VariableSizeResult, UInt32VariableSize};

/// A data file is a run of frames of this size, header included - the layout of the
/// working frames of archived-data.
pub const FRAME_LEN: usize = 32 * 1024;

/// `size u32 · next_file u8 · next u32 · used u16`
pub const FRAME_HEADER_LEN: usize = 11;

/// A frame of the data files of an hour: the number of its file and its offset in it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId {
    pub file: u8,
    pub offset: u32,
}

impl FrameId {
    /// The first frame of the first file: the head of the chain, never a `next`
    pub const HEAD: FrameId = FrameId { file: 0, offset: 0 };
}

pub const FRAME_CAPACITY: usize = FRAME_LEN - FRAME_HEADER_LEN;

/// The time an entry starts with: `i64` little-endian of microseconds.
pub const ENTRY_TIME_LEN: usize = 8;

/// The longest body of an entry - its time and its compressed block. A full frame then
/// always holds at least three entries, so it can be halved at an entry boundary and the
/// entry being inserted fits beside either half.
pub const MAX_ENTRY_BODY_LEN: usize = 8 * 1024;

/// One frame of a data file: a header, then `used` bytes of entries one after another.
/// `next` is the next frame of the chain - in this data file or in another one of the hour -
/// and none on the last one: the head of the chain, frame 0 of file 0, is never a `next`.
#[derive(Clone)]
pub struct Frame {
    bytes: Vec<u8>,
}

impl Frame {
    pub fn new() -> Self {
        let mut bytes = vec![0u8; FRAME_LEN];
        bytes[0..4].copy_from_slice(&(FRAME_LEN as u32).to_le_bytes());
        Self { bytes }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() != FRAME_LEN {
            return Err(format!(
                "A frame is {} bytes, and {} bytes are given",
                FRAME_LEN,
                bytes.len()
            ));
        }

        let frame = Self { bytes };

        let size = u32::from_le_bytes(frame.bytes[0..4].try_into().unwrap());
        if size as usize != FRAME_LEN {
            return Err(format!("A frame of size {} is met", size));
        }

        if frame.used() > FRAME_CAPACITY {
            return Err(format!(
                "A frame has {} bytes of entries, and there is room for {}",
                frame.used(),
                FRAME_CAPACITY
            ));
        }

        if let Some(next) = frame.next() {
            if next.offset as usize % FRAME_LEN != 0 {
                return Err(format!("A frame is chained to {:?}", next));
            }
        }

        Ok(frame)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn next(&self) -> Option<FrameId> {
        let next = FrameId {
            file: self.bytes[4],
            offset: u32::from_le_bytes(self.bytes[5..9].try_into().unwrap()),
        };

        (next != FrameId::HEAD).then_some(next)
    }

    pub fn set_next(&mut self, next: Option<FrameId>) {
        let next = next.unwrap_or(FrameId::HEAD);
        self.bytes[4] = next.file;
        self.bytes[5..9].copy_from_slice(&next.offset.to_le_bytes());
    }

    pub fn used(&self) -> usize {
        u16::from_le_bytes(self.bytes[9..11].try_into().unwrap()) as usize
    }

    fn set_used(&mut self, used: usize) {
        self.bytes[9..11].copy_from_slice(&(used as u16).to_le_bytes());
    }

    /// The offset from the start of the frame its entries end at
    pub fn records_end(&self) -> usize {
        FRAME_HEADER_LEN + self.used()
    }

    pub fn free(&self) -> usize {
        FRAME_CAPACITY - self.used()
    }

    pub fn entry_at(&self, offset: usize) -> Result<Entry<'_>, String> {
        parse_entry(&self.bytes[..self.records_end()], offset)
    }

    /// Where each entry of the frame starts
    pub fn entry_offsets(&self) -> Result<Vec<usize>, String> {
        let mut result = Vec::new();
        let mut offset = FRAME_HEADER_LEN;
        let end = self.records_end();

        while offset < end {
            let entry = self.entry_at(offset)?;
            result.push(offset);
            offset += entry.len;
        }

        Ok(result)
    }

    /// Puts `entry` at `offset`: the entries from `offset` on move up by its length
    pub fn insert(&mut self, offset: usize, entry: &[u8]) {
        let end = self.records_end();

        assert!(
            offset >= FRAME_HEADER_LEN && offset <= end && entry.len() <= self.free(),
            "An entry of {} bytes does not go into a frame with {} bytes free at {}",
            entry.len(),
            self.free(),
            offset
        );

        self.bytes.copy_within(offset..end, offset + entry.len());
        self.bytes[offset..offset + entry.len()].copy_from_slice(entry);
        self.set_used(self.used() + entry.len());
    }

    /// Takes the entry of `len` bytes at `offset` out: the entries after it move down
    pub fn remove(&mut self, offset: usize, len: usize) {
        let end = self.records_end();

        self.bytes.copy_within(offset + len..end, offset);
        self.bytes[end - len..end].fill(0);
        self.set_used(self.used() - len);
    }

    /// Takes the entries from `offset` on out of the frame; the room they leave is zeroed
    pub fn split_off(&mut self, offset: usize) -> Vec<u8> {
        let end = self.records_end();
        let result = self.bytes[offset..end].to_vec();

        self.bytes[offset..end].fill(0);
        self.set_used(offset - FRAME_HEADER_LEN);

        result
    }
}

/// An entry of a frame: `[varint length][time i64][compressed block]`, the length being
/// the length of the time and the block.
pub struct Entry<'s> {
    /// The whole entry, its length prefix included
    pub len: usize,
    pub time: i64,
    /// The compressed block
    pub payload: &'s [u8],
}

/// Reads the entry `records[offset..]` starts with. `records` ends where the entries of
/// its frame end.
pub fn parse_entry(records: &[u8], offset: usize) -> Result<Entry<'_>, String> {
    if offset >= records.len() {
        return Err(format!("There is no entry at {}", offset));
    }

    let (body_len, prefix_len) = match UInt32VariableSize::from_slice(&records[offset..]) {
        ParseUInt32VariableSizeResult::Ok { value, size } => (value.get_value() as usize, size),
        ParseUInt32VariableSizeResult::NotEnoughDataInBuffer(_) => {
            return Err(format!("The length of the entry at {} is cut", offset));
        }
    };

    if body_len < ENTRY_TIME_LEN || body_len > MAX_ENTRY_BODY_LEN {
        return Err(format!(
            "The entry at {} is {} bytes long",
            offset, body_len
        ));
    }

    let body_start = offset + prefix_len;
    let end = body_start + body_len;

    if end > records.len() {
        return Err(format!(
            "The entry at {} of {} bytes reaches past the entries of its frame",
            offset, body_len
        ));
    }

    let time = i64::from_le_bytes(
        records[body_start..body_start + ENTRY_TIME_LEN]
            .try_into()
            .unwrap(),
    );

    Ok(Entry {
        len: end - offset,
        time,
        payload: &records[body_start + ENTRY_TIME_LEN..end],
    })
}

pub fn encode_entry(time: i64, payload: &[u8]) -> Vec<u8> {
    let body_len = ENTRY_TIME_LEN + payload.len();

    let mut result = Vec::with_capacity(body_len + 4);
    UInt32VariableSize::new(body_len as u32).serialize(&mut result);
    result.extend_from_slice(&time.to_le_bytes());
    result.extend_from_slice(payload);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_inserted_and_split_off() {
        let mut frame = Frame::new();

        let first = encode_entry(1, b"first");
        let third = encode_entry(3, b"third");
        let second = encode_entry(2, &[7u8; 100]);

        frame.insert(FRAME_HEADER_LEN, &first);
        frame.insert(FRAME_HEADER_LEN + first.len(), &third);
        frame.insert(FRAME_HEADER_LEN + first.len(), &second);

        let offsets = frame.entry_offsets().unwrap();
        let times: Vec<i64> = offsets
            .iter()
            .map(|offset| frame.entry_at(*offset).unwrap().time)
            .collect();
        assert_eq!(times, [1, 2, 3]);
        assert_eq!(frame.entry_at(offsets[1]).unwrap().payload, &[7u8; 100]);

        let upper = frame.split_off(offsets[1]);
        assert_eq!(upper.len(), second.len() + third.len());
        assert_eq!(frame.entry_offsets().unwrap(), [FRAME_HEADER_LEN]);

        let frame = Frame::from_bytes(frame.as_bytes().to_vec()).unwrap();
        assert_eq!(frame.used(), first.len());
    }

    #[test]
    fn zeroed_bytes_are_not_an_entry() {
        assert!(parse_entry(&[0u8; 64], 0).is_err());
    }
}
