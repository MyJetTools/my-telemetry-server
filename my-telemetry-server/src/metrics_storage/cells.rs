use super::{FrameId, FRAME_LEN};

pub const SECONDS_PER_HOUR: usize = 3600;

const CELL_LEN: usize = 9;
const CELLS_LEN: usize = SECONDS_PER_HOUR * CELL_LEN;

/// No first entry: a second with no records
const NO_OFFSET: u32 = u32::MAX;

/// The cells - `file u8 · offset u32 · count u32` for each second: the data file of the
/// first entry of the second, the entry's offset in it, and how many records the second
/// has - then a byte which
/// says whether they agree with the data file. A flush marks the index dirty before it
/// touches the data file, and writes the cells and the clean mark back once it is done:
/// an index found dirty is rebuilt from the data file.
pub const INDEX_FILE_LEN: usize = CELLS_LEN + 1;
pub const INDEX_STATE_OFFSET: u64 = CELLS_LEN as u64;
pub const INDEX_DIRTY: u8 = 1;
const INDEX_CLEAN: u8 = 0;

/// Where the first entry of a second is: its frame and the entry's offset from the frame's
/// start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub frame: FrameId,
    pub offset: u32,
}

impl Cell {
    pub const EMPTY: Cell = Cell {
        frame: FrameId {
            file: u8::MAX,
            offset: u32::MAX,
        },
        offset: u32::MAX,
    };

    pub fn is_empty(&self) -> bool {
        self.offset == u32::MAX
    }

    /// A frame starts at a multiple of its length and an entry starts inside it, so the
    /// two make one offset in the file - never `NO_OFFSET`, as no entry starts in the last
    /// bytes of a frame.
    fn to_file_offset(self) -> (u8, u32) {
        if self.is_empty() {
            (0, NO_OFFSET)
        } else {
            (self.frame.file, self.frame.offset + self.offset)
        }
    }

    fn from_file_offset(file: u8, offset: u32) -> Self {
        if offset == NO_OFFSET {
            return Self::EMPTY;
        }

        let frame = offset - offset % FRAME_LEN as u32;

        Self {
            frame: FrameId {
                file,
                offset: frame,
            },
            offset: offset - frame,
        }
    }
}

/// The index of an hour: for every second of it, where its first entry is and how many
/// records it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourCells {
    cells: Vec<Cell>,
    counts: Vec<u32>,
}

impl HourCells {
    pub fn new() -> Self {
        Self {
            cells: vec![Cell::EMPTY; SECONDS_PER_HOUR],
            counts: vec![0; SECONDS_PER_HOUR],
        }
    }

    pub fn get(&self, second: usize) -> Cell {
        self.cells[second]
    }

    pub fn set(&mut self, second: usize, cell: Cell) {
        self.cells[second] = cell;
    }

    pub fn count(&self, second: usize) -> u32 {
        self.counts[second]
    }

    pub fn set_count(&mut self, second: usize, count: u32) {
        self.counts[second] = count;
    }

    /// The records of the hour
    pub fn total(&self) -> u64 {
        self.counts.iter().map(|count| *count as u64).sum()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Cell> {
        self.cells.iter_mut().filter(|cell| !cell.is_empty())
    }

    /// The first second from `second` on which has entries
    pub fn first_from(&self, second: usize) -> Option<(usize, Cell)> {
        (second..SECONDS_PER_HOUR)
            .map(|second| (second, self.cells[second]))
            .find(|(_, cell)| !cell.is_empty())
    }

    pub fn last(&self) -> Option<(usize, Cell)> {
        (0..SECONDS_PER_HOUR)
            .rev()
            .map(|second| (second, self.cells[second]))
            .find(|(_, cell)| !cell.is_empty())
    }

    pub fn to_index_file(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(INDEX_FILE_LEN);

        for (cell, count) in self.cells.iter().zip(&self.counts) {
            let (file, offset) = cell.to_file_offset();
            result.push(file);
            result.extend_from_slice(&offset.to_le_bytes());
            result.extend_from_slice(&count.to_le_bytes());
        }

        result.push(INDEX_CLEAN);
        result
    }

    /// `None` when the file is not a clean index
    pub fn from_index_file(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != INDEX_FILE_LEN || bytes[CELLS_LEN] != INDEX_CLEAN {
            return None;
        }

        let mut result = Self::new();

        for (second, chunk) in bytes[..CELLS_LEN].chunks_exact(CELL_LEN).enumerate() {
            let offset = u32::from_le_bytes(chunk[1..5].try_into().unwrap());
            result.cells[second] = Cell::from_file_offset(chunk[0], offset);
            result.counts[second] = u32::from_le_bytes(chunk[5..9].try_into().unwrap());
        }

        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_file_round_trip() {
        let mut cells = HourCells::new();
        cells.set(5, Cell { frame: FrameId::HEAD, offset: 11 });
        cells.set_count(5, 3);
        cells.set(
            3599,
            Cell {
                frame: FrameId {
                    file: 2,
                    offset: 32768 * 7,
                },
                offset: 200,
            },
        );
        cells.set_count(3599, 100_000);

        let bytes = cells.to_index_file();
        assert_eq!(bytes.len(), INDEX_FILE_LEN);
        assert_eq!(HourCells::from_index_file(&bytes), Some(cells.clone()));

        assert_eq!(cells.first_from(0).unwrap().0, 5);
        assert_eq!(cells.first_from(6).unwrap().0, 3599);
        assert_eq!(cells.last().unwrap().0, 3599);

        let mut dirty = bytes;
        dirty[INDEX_STATE_OFFSET as usize] = INDEX_DIRTY;
        assert_eq!(HourCells::from_index_file(&dirty), None);
    }
}
