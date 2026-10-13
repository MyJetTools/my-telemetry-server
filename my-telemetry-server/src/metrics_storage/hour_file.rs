use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use ahash::{AHashMap, AHashSet};

use super::{
    compress_records, decompress_records, time_of, Cell, CompressedBlock, DataFiles, DataSource,
    Frame, FrameId, HourCells, HourKeyIndex, HourProcessIndex, KeyIndex, KeyIndexDirectory,
    ProcessIndex, ProcessIndexDirectory, Rec, FRAME_CAPACITY, FRAME_HEADER_LEN, FRAME_LEN,
    INDEX_DIRTY, INDEX_STATE_OFFSET, SECONDS_PER_HOUR,
};

pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// A data file of an hour takes new frames until it is this long; then the next one does.
/// What is in it may still be rewritten in place, so it is a little longer at most.
pub const DATA_FILE_LEN: u64 = 3 * 1024 * 1024 * 1024;

/// Its number is a byte
pub const MAX_DATA_FILES: usize = 256;

const CHAIN_LOOP: &str = "The chain of frames returns to a frame it walked";

/// The key index is written at most this often while the hour is written into: it is kept
/// in memory, and one which did not get its last changes is rebuilt from the data files.
/// Not the 5 seconds of my-logger-server: here it is megabytes, written whole each time.
const KEY_INDEX_WRITE_INTERVAL: Duration = Duration::from_secs(60);

/// The process index is written once the hour has not been written into for this long. It
/// grows with the processes of the hour, not with its actions: written every minute, it
/// would be more bytes than the metrics themselves. An hour which is written all the time
/// keeps it in memory until it is over; after a crash it is rebuilt in the walk which
/// rebuilds the key index.
const PROCESS_INDEX_QUIET: Duration = Duration::from_secs(60);

/// The 3600 cells of the hour and the mark which says whether they agree with the data
pub const SECOND_INDEX_FILE_NAME: &str = "second.index";

/// How many records each action of each service has in each second
pub const KEY_INDEX_FILE_NAME: &str = "key.index";

/// Which seconds have events of each process
pub const PROCESS_INDEX_FILE_NAME: &str = "process.index";

/// `metrics.data00`, `metrics.data01` ... `metrics.dataFF`
pub fn data_file_name(file: u8) -> String {
    format!("metrics.data{:02X}", file)
}

/// The second of the hour starting at `hour_start` the time falls into
pub fn second_of(hour_start: i64, time: i64) -> Option<usize> {
    // A process id is asked too, and it is whatever its writer gave
    let second = time.checked_sub(hour_start)?.div_euclid(MICROS_PER_SECOND);

    if (0..SECONDS_PER_HOUR as i64).contains(&second) {
        Some(second as usize)
    } else {
        None
    }
}

/// Where an entry is, or goes: its frame and its offset from the frame's start
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Position {
    frame: FrameId,
    offset: usize,
}

impl Position {
    fn of(cell: Cell) -> Self {
        Self {
            frame: cell.frame,
            offset: cell.offset as usize,
        }
    }

    fn to_cell(self) -> Cell {
        Cell {
            frame: self.frame,
            offset: self.offset as u32,
        }
    }
}

/// A data file of the hour
struct DataFile {
    /// Once it exists
    file: Option<File>,
    /// Its frames, flushed or not
    frames_count: u32,
    /// Its frames on disk
    frames_on_disk: u32,
}

/// The files of one hour as their writer keeps them: the cells in memory, the frames an
/// insert has read or changed, and the changes until they are flushed.
pub struct HourFile {
    hour_start: i64,
    /// The folder of the hour
    dir: PathBuf,
    index_path: PathBuf,
    /// Once it exists
    index_file: Option<File>,
    data_files: Vec<DataFile>,
    /// A data file takes new frames until it is this long
    data_file_len: u64,
    cells: HourCells,
    /// The last frame of the chain
    tail: Option<FrameId>,
    last_second: Option<usize>,
    frames: AHashMap<FrameId, Frame>,
    /// Frames changed since the last flush, in the order they are to be written
    dirty: Vec<FrameId>,
    index_clean_on_disk: bool,
    /// Uploaded, or being uploaded: nothing is written into it any more
    sealed: bool,
    key_index_path: PathBuf,
    key_index: KeyIndexState,
    key_index_changed: bool,
    key_index_written: Option<Instant>,
    process_index_path: PathBuf,
    process_index: ProcessIndexState,
    /// When it was last changed, while its file does not have the change
    process_index_changed: Option<Instant>,
}

enum KeyIndexState {
    /// The hour is written into: the whole index, kept up to date by the writes
    Full(Arc<KeyIndex>),
    /// The hour is read: only the page of its keys and values
    Directory(Arc<KeyIndexDirectory>),
}

enum ProcessIndexState {
    /// The hour is written into: the whole index, kept up to date by the writes
    Full(ProcessIndex),
    /// The hour is read: only the first pair of each of its pages
    Directory(Arc<ProcessIndexDirectory>),
}

impl HourFile {
    /// The files of the hour which starts at `hour_start`, in its folder `dir`. Nothing is
    /// created until the first flush. An index which is not clean - its flush did not
    /// complete - is rebuilt from the data files, and so are a key index and a process
    /// index which do not count the records the index does. The two of an hour to be
    /// written - `full_indexes` - are read whole, of any other only their directories.
    pub fn open(dir: &Path, hour_start: i64, full_indexes: bool) -> std::io::Result<Self> {
        let mut result = Self {
            hour_start,
            dir: dir.to_path_buf(),
            index_path: dir.join(SECOND_INDEX_FILE_NAME),
            index_file: None,
            data_files: Vec::new(),
            data_file_len: DATA_FILE_LEN,
            cells: HourCells::new(),
            tail: None,
            last_second: None,
            frames: AHashMap::new(),
            dirty: Vec::new(),
            index_clean_on_disk: false,
            sealed: false,
            key_index_path: dir.join(KEY_INDEX_FILE_NAME),
            key_index: KeyIndexState::Full(Arc::new(KeyIndex::new())),
            key_index_changed: false,
            key_index_written: None,
            process_index_path: dir.join(PROCESS_INDEX_FILE_NAME),
            process_index: ProcessIndexState::Full(ProcessIndex::new(hour_start)),
            process_index_changed: None,
        };

        // A file which ends inside a frame did not get the whole of it
        let mut cut = false;

        for file in 0..MAX_DATA_FILES {
            let path = dir.join(data_file_name(file as u8));

            if !path.exists() {
                break;
            }

            let data_file = OpenOptions::new().read(true).write(true).open(&path)?;
            let len = data_file.metadata()?.len();

            cut |= len % FRAME_LEN as u64 != 0;

            let frames = (len / FRAME_LEN as u64) as u32;
            result.data_files.push(DataFile {
                file: Some(data_file),
                frames_count: frames,
                frames_on_disk: frames,
            });
        }

        if result.data_files.is_empty() {
            return Ok(result);
        }

        let index_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&result.index_path)?;

        let mut index_bytes = Vec::new();
        (&index_file).read_to_end(&mut index_bytes)?;

        result.index_file = Some(index_file);

        // A rebuild of the cells reads every record: it counts the key index and the
        // process index as it goes
        let mut rebuilt_indexes = None;

        match HourCells::from_index_file(&index_bytes) {
            Some(cells) if !cut => {
                result.cells = cells;
                result.index_clean_on_disk = true;
            }
            _ => {
                println!(
                    "The index of {} is not clean: it is rebuilt from the data files",
                    result.name()
                );
                rebuilt_indexes = Some(result.rebuild()?);
            }
        }

        result.tail = match result.find_tail() {
            Ok(tail) => tail,
            // A clean index over data files which do not read to their end: what a power
            // loss leaves, as nothing is fsync'ed. The chain is cut where it stops reading.
            Err(err) if rebuilt_indexes.is_none() => {
                println!(
                    "The data files of {} do not read to their end: {}. The index is rebuilt from them",
                    result.name(),
                    err
                );
                rebuilt_indexes = Some(result.rebuild()?);
                result.find_tail()?
            }
            Err(err) => return Err(err),
        };

        result.last_second = result.cells.last().map(|(second, _)| second);

        if let Some((key_index, process_index)) = rebuilt_indexes {
            result.take_rebuilt(Some(key_index), Some(process_index))?;
            return Ok(result);
        }

        let total = result.cells.total();

        let key_index = if full_indexes {
            result
                .read_full_key_index()
                .map(|index| KeyIndexState::Full(Arc::new(index)))
        } else {
            KeyIndexDirectory::read_local(&result.key_index_path)
                .filter(|directory| directory.total_records() == total)
                .map(|directory| KeyIndexState::Directory(Arc::new(directory)))
        };

        let process_index = if full_indexes {
            result.read_full_process_index().map(ProcessIndexState::Full)
        } else {
            ProcessIndexDirectory::read_local(hour_start, &result.process_index_path)
                .filter(|directory| directory.total_records() == total)
                .map(|directory| ProcessIndexState::Directory(Arc::new(directory)))
        };

        // One walk of the data files counts both: each is taken from it when its file
        // does not count the records the index does
        let (rebuilt_key_index, rebuilt_process_index) =
            if key_index.is_none() || process_index.is_none() {
                println!(
                    "{}: {} rebuilt from the data files",
                    result.name(),
                    match (&key_index, &process_index) {
                        (None, None) => "the key index and the process index are",
                        (None, Some(_)) => "the key index is",
                        _ => "the process index is",
                    }
                );

                match result.rebuild_indexes() {
                    Ok((rebuilt_key_index, rebuilt_process_index))
                        if rebuilt_key_index.total_records() == total =>
                    {
                        (
                            key_index.is_none().then_some(rebuilt_key_index),
                            process_index.is_none().then_some(rebuilt_process_index),
                        )
                    }
                    // Data which does not read under a clean index - what a power loss
                    // leaves in the middle of the chain: an entry which is not one, or a
                    // block which does not decompress and so has fewer records than the
                    // index counts. The index is rebuilt as it is when it is not clean:
                    // the chain is cut at such an entry, such a block counts for nothing.
                    // An hour which opens with what reads - and whose indexes agree with
                    // it from then on - not one which does not open.
                    walked => {
                        println!(
                            "The data files of {} do not read: {}. The index is rebuilt from them",
                            result.name(),
                            match walked {
                                Ok(_) => "a block has not the records the index counts".to_string(),
                                Err(err) => err.to_string(),
                            }
                        );

                        let (rebuilt_key_index, rebuilt_process_index) = result.rebuild()?;
                        result.tail = result.find_tail()?;
                        result.last_second = result.cells.last().map(|(second, _)| second);

                        result.take_rebuilt(Some(rebuilt_key_index), Some(rebuilt_process_index))?;
                        return Ok(result);
                    }
                }
            } else {
                (None, None)
            };

        if let Some(key_index) = key_index {
            result.key_index = key_index;
        }

        if let Some(process_index) = process_index {
            result.process_index = process_index;
        }

        result.take_rebuilt(rebuilt_key_index, rebuilt_process_index)?;

        Ok(result)
    }

    /// The indexes a walk of the data files counted take the place of the ones of the
    /// files, and are written
    fn take_rebuilt(
        &mut self,
        key_index: Option<KeyIndex>,
        process_index: Option<ProcessIndex>,
    ) -> std::io::Result<()> {
        if let Some(key_index) = key_index {
            self.key_index = KeyIndexState::Full(Arc::new(key_index));
            self.key_index_changed = true;
            self.write_key_index()?;
        }

        if let Some(process_index) = process_index {
            self.process_index = ProcessIndexState::Full(process_index);
            self.process_index_changed = Some(Instant::now());
            self.write_process_index()?;
        }

        Ok(())
    }

    /// For the messages
    fn name(&self) -> String {
        self.dir.display().to_string()
    }

    /// A data file to take new frames until it is `len` long - a small one tests going
    /// over to the next file
    #[cfg(test)]
    pub fn set_data_file_len(&mut self, len: u64) {
        self.data_file_len = len;
    }

    /// The key index file whole, when it counts the records the index does
    fn read_full_key_index(&self) -> Option<KeyIndex> {
        std::fs::read(&self.key_index_path)
            .ok()
            .and_then(|bytes| KeyIndex::from_file(&bytes))
            .filter(|index| index.total_records() == self.cells.total())
    }

    /// The process index file whole, when it counts the records the index does
    fn read_full_process_index(&self) -> Option<ProcessIndex> {
        std::fs::read(&self.process_index_path)
            .ok()
            .and_then(|bytes| ProcessIndex::from_file(self.hour_start, &bytes))
            .filter(|index| index.total_records() == self.cells.total())
    }

    /// The process index as a trace uses it
    pub fn process_index(&self) -> HourProcessIndex<'_> {
        match &self.process_index {
            ProcessIndexState::Full(index) => HourProcessIndex::Full(index),
            ProcessIndexState::Directory(directory) => HourProcessIndex::Directory {
                directory: directory.clone(),
                source: DataSource::Local(self.process_index_path.clone()),
            },
        }
    }

    /// The whole process index, to be written into: read when only its directory was
    fn full_process_index(&mut self) -> Result<&mut ProcessIndex, String> {
        if let ProcessIndexState::Directory(_) = self.process_index {
            let index = match self.read_full_process_index() {
                Some(index) => index,
                None => self.rebuild_indexes().map_err(|err| err.to_string())?.1,
            };

            self.process_index = ProcessIndexState::Full(index);
        }

        match &mut self.process_index {
            ProcessIndexState::Full(index) => Ok(index),
            ProcessIndexState::Directory(_) => unreachable!(),
        }
    }

    /// The whole process index, as the files have it
    #[cfg(test)]
    pub fn whole_process_index(&self) -> ProcessIndex {
        match &self.process_index {
            ProcessIndexState::Full(index) => index.clone(),
            ProcessIndexState::Directory(_) => self.read_full_process_index().unwrap(),
        }
    }

    /// The key index as a search uses it
    pub fn key_index(&self) -> HourKeyIndex {
        match &self.key_index {
            KeyIndexState::Full(index) => HourKeyIndex::Full(index.clone()),
            KeyIndexState::Directory(directory) => HourKeyIndex::Directory {
                directory: directory.clone(),
                source: DataSource::Local(self.key_index_path.clone()),
            },
        }
    }

    /// The whole key index, to be written into: read when only its page was
    fn full_key_index(&mut self) -> Result<&mut KeyIndex, String> {
        if let KeyIndexState::Directory(_) = self.key_index {
            let index = match self.read_full_key_index() {
                Some(index) => index,
                None => self.rebuild_indexes().map_err(|err| err.to_string())?.0,
            };

            self.key_index = KeyIndexState::Full(Arc::new(index));
        }

        match &mut self.key_index {
            // A search which still holds the index gets a copy made of it here: searches
            // let go of it before the hour is written into again
            KeyIndexState::Full(index) => Ok(Arc::make_mut(index)),
            KeyIndexState::Directory(_) => unreachable!(),
        }
    }

    /// The whole key index, as the files have it
    #[cfg(test)]
    pub fn whole_key_index(&self) -> KeyIndex {
        match &self.key_index {
            KeyIndexState::Full(index) => index.as_ref().clone(),
            KeyIndexState::Directory(_) => self.read_full_key_index().unwrap(),
        }
    }

    #[cfg(test)]
    pub fn hour_start(&self) -> i64 {
        self.hour_start
    }

    pub fn cells(&self) -> &HourCells {
        &self.cells
    }

    /// Where a search reads the data files of the hour
    pub fn data_files(&self) -> DataFiles {
        DataFiles::Local {
            dir: self.dir.clone(),
        }
    }

    pub fn has_entries(&self) -> bool {
        self.tail.is_some()
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    pub fn set_sealed(&mut self, sealed: bool) {
        self.sealed = sealed;
    }

    fn frames_count(&self) -> u64 {
        self.data_files
            .iter()
            .map(|file| file.frames_count as u64)
            .sum()
    }

    /// Writes the records of one of its seconds, in time order. `blocks` are the records
    /// compressed - by the caller, before it took the lock of the hour - or `None` to have
    /// it done here. A second which has entries already gets the records merged into its
    /// last block: the block is decompressed, the records are added and it is compressed
    /// again - into two blocks when it outgrows one - and it takes the place of the old one.
    ///
    /// On an error the hour is not to be flushed: what is in memory of it is let go, and
    /// it is opened again from its files.
    pub fn write_second(
        &mut self,
        second: usize,
        records: &[Rec],
        blocks: Option<Vec<CompressedBlock>>,
    ) -> Result<(), String> {
        if records.is_empty() {
            return Ok(());
        }

        self.full_key_index()?.add_records(second, records);
        self.key_index_changed = true;

        self.full_process_index()?.add_records(second, records);
        self.process_index_changed = Some(Instant::now());

        let mut to_remove = None;
        // The records of the second which are in the block taken out
        let mut removed_records = 0;

        let (mut place, first_of_second, blocks) = match self.last_entry_of(second)? {
            Some((last, len)) => match self.decompress_at(last) {
                Ok(mut merged) => {
                    removed_records = merged.len();
                    merged.extend_from_slice(records);
                    merged.sort_by_key(time_of);

                    to_remove = Some((last, len));
                    (
                        Some(last),
                        self.cells.get(second) == last.to_cell(),
                        compress_records(&merged),
                    )
                }
                Err(err) => {
                    println!(
                        "The last block of second {} of {} does not read: {}. The records go into a block of their own",
                        second,
                        self.name(),
                        err
                    );

                    let after = Position {
                        frame: last.frame,
                        offset: last.offset + len,
                    };
                    let blocks = blocks.unwrap_or_else(|| compress_records(records));
                    (Some(after), false, blocks)
                }
            },
            None => {
                let blocks = blocks.unwrap_or_else(|| compress_records(records));
                (self.place_for_new_second(second)?, true, blocks)
            }
        };

        // Each entry takes at most two frames more: decided before anything changes
        if self.room_for_frames() < 2 * blocks.len() as u64 {
            return Err(format!(
                "The data files of {} have no room for more frames",
                self.name()
            ));
        }

        if let Some((position, len)) = to_remove {
            self.remove_entry(position, len);
        }

        for (index, block) in blocks.iter().enumerate() {
            let cell = self.insert_at(place, &block.entry)?;

            if index == 0 && first_of_second {
                self.cells.set(second, cell);
            }

            place = Some(Position {
                frame: cell.frame,
                offset: cell.offset as usize + block.entry.len(),
            });
        }

        let written: usize = blocks.iter().map(|block| block.records).sum();
        let count = self.cells.count(second) as usize - removed_records + written;
        self.cells.set_count(second, count as u32);

        // A record which fits no entry - cut as it is - is dropped as the blocks are
        // made: it is not one of the records the indexes are to count
        let dropped = (removed_records + records.len()).saturating_sub(written);

        if dropped > 0 {
            self.full_key_index()?.forget_records(dropped);
            self.full_process_index()?.forget_records(dropped);
        }

        self.last_second = Some(self.last_second.map_or(second, |last| last.max(second)));

        Ok(())
    }

    /// Writes every changed frame, then the cells; the key index when it is changed and
    /// it is time, the process index when the hour has not been written into for a while -
    /// or both when `force_indexes`.
    pub fn flush(&mut self, force_indexes: bool) -> std::io::Result<()> {
        if !self.dirty.is_empty() {
            self.flush_frames()?;
        }

        let due = self
            .key_index_written
            .map_or(true, |written| written.elapsed() >= KEY_INDEX_WRITE_INTERVAL);

        if self.key_index_changed && (due || force_indexes) {
            self.write_key_index()?;
        }

        let quiet = self
            .process_index_changed
            .map_or(false, |changed| changed.elapsed() >= PROCESS_INDEX_QUIET);

        if quiet || (force_indexes && self.process_index_changed.is_some()) {
            self.write_process_index()?;
        }

        Ok(())
    }

    /// Next to the file and renamed over it: a key index is whole, or it is the one before
    fn write_key_index(&mut self) -> std::io::Result<()> {
        let KeyIndexState::Full(index) = &self.key_index else {
            // Only its page is read: nothing is changed
            return Ok(());
        };

        let mut temp = self.key_index_path.clone().into_os_string();
        temp.push(".tmp");

        std::fs::write(&temp, index.to_file())?;
        std::fs::rename(&temp, &self.key_index_path)?;

        self.key_index_changed = false;
        self.key_index_written = Some(Instant::now());

        Ok(())
    }

    /// Next to the file and renamed over it, as the key index is
    fn write_process_index(&mut self) -> std::io::Result<()> {
        let ProcessIndexState::Full(index) = &self.process_index else {
            // Only its directory is read: nothing is changed
            return Ok(());
        };

        let mut temp = self.process_index_path.clone().into_os_string();
        temp.push(".tmp");

        std::fs::write(&temp, index.to_file())?;
        std::fs::rename(&temp, &self.process_index_path)?;

        self.process_index_changed = None;

        Ok(())
    }

    /// The index is marked dirty before the first frame goes out, so a flush which does
    /// not complete leaves an index that is rebuilt on the next open.
    fn flush_frames(&mut self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;

        if self.index_file.is_none() {
            let index_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&self.index_path)?;

            self.index_file = Some(index_file);
        }

        for (number, data_file) in self.data_files.iter_mut().enumerate() {
            if data_file.file.is_none() {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(self.dir.join(data_file_name(number as u8)))?;

                data_file.file = Some(file);
            }
        }

        let index_file = self.index_file.as_ref().unwrap();

        if self.index_clean_on_disk {
            index_file.write_all_at(&[INDEX_DIRTY], INDEX_STATE_OFFSET)?;
            self.index_clean_on_disk = false;
        }

        // The new frames first: nothing points at them until the changed ones are written
        let data_files = &self.data_files;
        let (mut new_frames, changed_frames): (Vec<FrameId>, Vec<FrameId>) =
            self.dirty.iter().partition(|frame| {
                frame.offset / FRAME_LEN as u32 >= data_files[frame.file as usize].frames_on_disk
            });
        new_frames.sort();

        for frame in new_frames.iter().chain(changed_frames.iter()) {
            self.data_files[frame.file as usize]
                .file
                .as_ref()
                .unwrap()
                .write_all_at(self.frames[frame].as_bytes(), frame.offset as u64)?;
        }

        index_file.write_all_at(&self.cells.to_index_file(), 0)?;
        self.index_clean_on_disk = true;

        for data_file in &mut self.data_files {
            data_file.frames_on_disk = data_file.frames_count;
        }

        self.dirty.clear();

        // The last frame is kept: it is where the next seconds go
        let tail = self.tail;
        self.frames.retain(|frame, _| Some(*frame) == tail);

        Ok(())
    }

    /// Writes the chain into new files frame after frame, each one filled before the next
    /// one starts and a file going over to the next one where the data files of the hour
    /// do: what is uploaded is read as one run. Expects the hour to be flushed. Returns how
    /// many data files are written.
    pub fn compact_to(
        &self,
        data_path: impl Fn(u8) -> PathBuf,
        index_path: &Path,
        key_index_path: &Path,
        process_index_path: &Path,
    ) -> std::io::Result<usize> {
        let mut data = std::io::BufWriter::new(File::create(data_path(0))?);
        let mut files: usize = 1;
        let mut cells = HourCells::new();

        if self.tail.is_some() {
            let mut out = Frame::new();
            let mut out_frame = FrameId::HEAD;
            let mut frame_id = FrameId::HEAD;
            let mut hops = 0;

            loop {
                let frame = self.read_frame(frame_id)?;
                let mut offset = FRAME_HEADER_LEN;

                while offset < frame.records_end() {
                    let entry = frame.entry_at(offset).map_err(corrupted)?;

                    if out.free() < entry.len {
                        let next = self.compact_frame_after(out_frame)?;
                        out.set_next(Some(next));
                        data.write_all(out.as_bytes())?;

                        if next.file != out_frame.file {
                            data.flush()?;
                            data = std::io::BufWriter::new(File::create(data_path(next.file))?);
                            files += 1;
                        }

                        out_frame = next;
                        out = Frame::new();
                    }

                    let at = out.records_end();
                    out.insert(at, &frame.as_bytes()[offset..offset + entry.len]);

                    if let Some(second) = second_of(self.hour_start, entry.time) {
                        if cells.get(second).is_empty() {
                            cells.set(
                                second,
                                Cell {
                                    frame: out_frame,
                                    offset: at as u32,
                                },
                            );
                        }
                    }

                    offset += entry.len;
                }

                let Some(next) = frame.next() else {
                    break;
                };

                hops += 1;
                if hops > self.frames_count() {
                    return Err(corrupted(CHAIN_LOOP));
                }

                frame_id = next;
            }

            if out.used() > 0 {
                data.write_all(out.as_bytes())?;
            }
        }

        for second in 0..SECONDS_PER_HOUR {
            cells.set_count(second, self.cells.count(second));
        }

        data.flush()?;
        std::fs::write(index_path, cells.to_index_file())?;
        match &self.key_index {
            KeyIndexState::Full(index) => std::fs::write(key_index_path, index.to_file())?,
            KeyIndexState::Directory(_) => {
                std::fs::copy(&self.key_index_path, key_index_path)?;
            }
        }
        // It names seconds, not places: the copy has the same one
        match &self.process_index {
            ProcessIndexState::Full(index) => {
                std::fs::write(process_index_path, index.to_file())?
            }
            ProcessIndexState::Directory(_) => {
                std::fs::copy(&self.process_index_path, process_index_path)?;
            }
        }

        Ok(files)
    }

    /// The frame of a compact copy after `frame`: the next one in its file, or the first
    /// one of the next file when the file is as long as a data file grows
    fn compact_frame_after(&self, frame: FrameId) -> std::io::Result<FrameId> {
        let next_offset = frame.offset as u64 + FRAME_LEN as u64;

        if next_offset < self.data_file_len {
            return Ok(FrameId {
                file: frame.file,
                offset: next_offset as u32,
            });
        }

        if frame.file as usize + 1 >= MAX_DATA_FILES {
            return Err(corrupted(format!(
                "The copy of {} needs more than {} data files",
                self.name(),
                MAX_DATA_FILES
            )));
        }

        Ok(FrameId {
            file: frame.file + 1,
            offset: 0,
        })
    }

    /// The last entry of the second and its length
    fn last_entry_of(&mut self, second: usize) -> Result<Option<(Position, usize)>, String> {
        let cell = self.cells.get(second);

        if cell.is_empty() {
            return Ok(None);
        }

        let second_end = self.hour_start + (second as i64 + 1) * MICROS_PER_SECOND;
        let mut position = Position::of(cell);
        let mut last = None;

        for _ in 0..=self.frames_count() {
            let frame = self.frame(position.frame)?;

            while position.offset < frame.records_end() {
                let entry = frame.entry_at(position.offset)?;

                if entry.time >= second_end {
                    return Ok(last);
                }

                last = Some((position, entry.len));
                position.offset += entry.len;
            }

            let Some(next) = frame.next() else {
                return Ok(last);
            };

            position = Position {
                frame: next,
                offset: FRAME_HEADER_LEN,
            };
        }

        Err(CHAIN_LOOP.to_string())
    }

    fn decompress_at(&mut self, position: Position) -> Result<Vec<Rec>, String> {
        let frame = self.frame(position.frame)?;
        let entry = frame.entry_at(position.offset)?;
        decompress_records(entry.payload)
    }

    /// A second with no entries goes before the first entry of the nearest later second
    /// which has one, at the end of the chain otherwise. `None` - the hour has no entries.
    fn place_for_new_second(&mut self, second: usize) -> Result<Option<Position>, String> {
        let later = match self.last_second {
            Some(last) if last > second => self.cells.first_from(second + 1),
            _ => None,
        };

        if let Some((_, cell)) = later {
            return Ok(Some(Position::of(cell)));
        }

        match self.tail {
            Some(tail) => {
                let offset = self.frame(tail)?.records_end();
                Ok(Some(Position {
                    frame: tail,
                    offset,
                }))
            }
            None => Ok(None),
        }
    }

    /// Puts the entry at the place: into its frame while it has room, the entries after
    /// the place moving up; into a new frame at the end of the chain; into one half of the
    /// frame, halved, when it is full.
    fn insert_at(&mut self, place: Option<Position>, entry: &[u8]) -> Result<Cell, String> {
        let Some(mut place) = place else {
            let frame = self.allocate_frame()?;
            self.frames
                .get_mut(&frame)
                .unwrap()
                .insert(FRAME_HEADER_LEN, entry);
            self.mark_dirty(frame);
            self.tail = Some(frame);

            return Ok(Cell {
                frame,
                offset: FRAME_HEADER_LEN as u32,
            });
        };

        for _ in 0..=self.frames_count() {
            let (free, records_end, next) = {
                let frame = self.frame(place.frame)?;
                (frame.free(), frame.records_end(), frame.next())
            };

            if entry.len() <= free {
                return Ok(self.insert_into_frame(place, entry));
            }

            if place.offset < records_end {
                return self.halve_and_insert(place, entry);
            }

            let Some(next) = next else {
                return self.append_frame(place.frame, entry);
            };

            // The end of a full frame is the start of the next one
            place = Position {
                frame: next,
                offset: FRAME_HEADER_LEN,
            };
        }

        Err(CHAIN_LOOP.to_string())
    }

    fn insert_into_frame(&mut self, place: Position, entry: &[u8]) -> Cell {
        self.frames
            .get_mut(&place.frame)
            .unwrap()
            .insert(place.offset, entry);

        for cell in self.cells.iter_mut() {
            if cell.frame == place.frame && cell.offset as usize >= place.offset {
                cell.offset += entry.len() as u32;
            }
        }

        self.mark_dirty(place.frame);
        place.to_cell()
    }

    fn append_frame(&mut self, last: FrameId, entry: &[u8]) -> Result<Cell, String> {
        let frame = self.allocate_frame()?;

        self.frames
            .get_mut(&frame)
            .unwrap()
            .insert(FRAME_HEADER_LEN, entry);
        self.frames.get_mut(&last).unwrap().set_next(Some(frame));

        self.mark_dirty(frame);
        self.mark_dirty(last);
        self.tail = Some(frame);

        Ok(Cell {
            frame,
            offset: FRAME_HEADER_LEN as u32,
        })
    }

    /// The frame is full and the place is before one of its entries. The frame is halved at
    /// the entry boundary nearest the middle of its entries, ties to the lower one; the
    /// entry goes into the lower half when its place is at or before the boundary. The
    /// upper half moves in front of the entries of the next frame when that one has room
    /// for it, into a new frame chained between the two otherwise - in the data file new
    /// frames go to, whichever file the halved one is in.
    fn halve_and_insert(&mut self, place: Position, entry: &[u8]) -> Result<Cell, String> {
        let next = self.frames[&place.frame].next();

        // Everything the insert touches is read before anything changes
        if let Some(next) = next {
            self.frame(next)?;
        }

        let (boundary, records_end) = {
            let frame = &self.frames[&place.frame];
            let middle = FRAME_HEADER_LEN + frame.used() / 2;

            let boundary = frame
                .entry_offsets()?
                .into_iter()
                .skip(1)
                .min_by_key(|boundary| (boundary.abs_diff(middle), *boundary))
                .ok_or_else(|| {
                    format!("The full frame {:?} has a single entry", place.frame)
                })?;

            (boundary, frame.records_end())
        };

        let into_lower = place.offset <= boundary;
        let lower_len = boundary - FRAME_HEADER_LEN + if into_lower { entry.len() } else { 0 };
        let upper_len = records_end - boundary + if into_lower { 0 } else { entry.len() };

        if lower_len > FRAME_CAPACITY || upper_len > FRAME_CAPACITY {
            return Err(format!(
                "An entry of {} bytes fits neither half of the frame {:?}",
                entry.len(),
                place.frame
            ));
        }

        let into_next = next.map_or(false, |next| self.frames[&next].free() >= upper_len);

        if !into_next && self.room_for_frames() == 0 {
            return Err(format!(
                "The data files of {} have no room for more frames",
                self.name()
            ));
        }

        // Nothing fails from here on
        let moved: Vec<usize> = (0..SECONDS_PER_HOUR)
            .filter(|second| {
                let cell = self.cells.get(*second);
                !cell.is_empty() && cell.frame == place.frame && cell.offset as usize >= boundary
            })
            .collect();

        let mut upper = self
            .frames
            .get_mut(&place.frame)
            .unwrap()
            .split_off(boundary);

        let mut entry_cell = None;

        if into_lower {
            self.frames
                .get_mut(&place.frame)
                .unwrap()
                .insert(place.offset, entry);

            for cell in self.cells.iter_mut() {
                let offset = cell.offset as usize;
                if cell.frame == place.frame && offset >= place.offset && offset < boundary {
                    cell.offset += entry.len() as u32;
                }
            }

            entry_cell = Some(place.to_cell());
        } else {
            let after = upper.split_off(place.offset - boundary);
            upper.extend_from_slice(entry);
            upper.extend_from_slice(&after);
        }

        let destination = match next.filter(|_| into_next) {
            Some(next) => {
                for cell in self.cells.iter_mut() {
                    if cell.frame == next {
                        cell.offset += upper.len() as u32;
                    }
                }

                self.frames
                    .get_mut(&next)
                    .unwrap()
                    .insert(FRAME_HEADER_LEN, &upper);
                self.mark_dirty(next);
                next
            }
            None => {
                let frame = self.allocate_frame()?;

                let new_frame = self.frames.get_mut(&frame).unwrap();
                new_frame.insert(FRAME_HEADER_LEN, &upper);
                new_frame.set_next(next);

                self.frames
                    .get_mut(&place.frame)
                    .unwrap()
                    .set_next(Some(frame));

                if next.is_none() {
                    self.tail = Some(frame);
                }

                self.mark_dirty(frame);
                frame
            }
        };

        self.mark_dirty(place.frame);

        for second in moved {
            let mut at = self.cells.get(second).offset as usize - boundary;

            if !into_lower && at >= place.offset - boundary {
                at += entry.len();
            }

            self.cells.set(
                second,
                Cell {
                    frame: destination,
                    offset: (FRAME_HEADER_LEN + at) as u32,
                },
            );
        }

        Ok(entry_cell.unwrap_or_else(|| Cell {
            frame: destination,
            offset: (FRAME_HEADER_LEN + place.offset - boundary) as u32,
        }))
    }

    /// Takes the entry out of its frame: the entries after it move down. The frame is in
    /// memory - the entry was found in it.
    fn remove_entry(&mut self, position: Position, len: usize) {
        self.frames
            .get_mut(&position.frame)
            .unwrap()
            .remove(position.offset, len);

        for cell in self.cells.iter_mut() {
            if cell.frame == position.frame && cell.offset as usize > position.offset {
                cell.offset -= len as u32;
            }
        }

        self.mark_dirty(position.frame);
    }

    /// How many frames more the data files can take
    fn room_for_frames(&self) -> u64 {
        let per_file = self.data_file_len.div_ceil(FRAME_LEN as u64);

        let in_last = self
            .data_files
            .last()
            .map_or(0, |last| per_file.saturating_sub(last.frames_count as u64));

        in_last + (MAX_DATA_FILES - self.data_files.len()) as u64 * per_file
    }

    /// A new frame goes into the last data file until it is `data_file_len` long, into a
    /// new one then
    fn allocate_frame(&mut self) -> Result<FrameId, String> {
        let last_is_full = self.data_files.last().map_or(true, |last| {
            last.frames_count as u64 * FRAME_LEN as u64 >= self.data_file_len
        });

        if last_is_full {
            if self.data_files.len() >= MAX_DATA_FILES {
                return Err(format!(
                    "{} has {} data files, and there is no room for more",
                    self.name(),
                    MAX_DATA_FILES
                ));
            }

            self.data_files.push(DataFile {
                file: None,
                frames_count: 0,
                frames_on_disk: 0,
            });
        }

        let file = self.data_files.len() - 1;
        let data_file = &mut self.data_files[file];

        let frame = FrameId {
            file: file as u8,
            offset: data_file.frames_count * FRAME_LEN as u32,
        };

        data_file.frames_count += 1;
        self.frames.insert(frame, Frame::new());

        Ok(frame)
    }

    fn mark_dirty(&mut self, frame: FrameId) {
        if !self.dirty.contains(&frame) {
            self.dirty.push(frame);
        }
    }

    /// The frame, read into memory for writing
    fn frame(&mut self, frame: FrameId) -> Result<&mut Frame, String> {
        if !self.frames.contains_key(&frame) {
            let read = self
                .read_frame_from_disk(frame)
                .map_err(|err| err.to_string())?;
            self.frames.insert(frame, read);
        }

        Ok(self.frames.get_mut(&frame).unwrap())
    }

    /// The frame, without keeping it in memory
    fn read_frame(&self, frame: FrameId) -> std::io::Result<Frame> {
        match self.frames.get(&frame) {
            Some(frame) => Ok(frame.clone()),
            None => self.read_frame_from_disk(frame),
        }
    }

    fn read_frame_from_disk(&self, frame: FrameId) -> std::io::Result<Frame> {
        let file = self
            .data_files
            .get(frame.file as usize)
            .filter(|data_file| {
                frame.offset % FRAME_LEN as u32 == 0
                    && (frame.offset / FRAME_LEN as u32) < data_file.frames_on_disk
            })
            .and_then(|data_file| data_file.file.as_ref())
            .ok_or_else(|| corrupted(format!("{} has no frame {:?}", self.name(), frame)))?;

        let mut bytes = vec![0u8; FRAME_LEN];
        file.read_exact_at(&mut bytes, frame.offset as u64)?;

        Frame::from_bytes(bytes).map_err(corrupted)
    }

    fn find_tail(&self) -> std::io::Result<Option<FrameId>> {
        if self.frames_count() == 0 {
            return Ok(None);
        }

        let mut frame = self
            .cells
            .last()
            .map(|(_, cell)| cell.frame)
            .unwrap_or(FrameId::HEAD);

        for _ in 0..=self.frames_count() {
            match self.read_frame(frame)?.next() {
                Some(next) => frame = next,
                None => return Ok(Some(frame)),
            }
        }

        Err(corrupted(CHAIN_LOOP))
    }

    /// Counts the actions and the processes of every record of the chain again
    fn rebuild_indexes(&self) -> std::io::Result<(KeyIndex, ProcessIndex)> {
        let mut result = (KeyIndex::new(), ProcessIndex::new(self.hour_start));

        if self.tail.is_none() {
            return Ok(result);
        }

        let mut frame_id = FrameId::HEAD;

        for _ in 0..=self.frames_count() {
            let frame = self.read_frame(frame_id)?;
            let mut offset = FRAME_HEADER_LEN;

            while offset < frame.records_end() {
                let entry = frame.entry_at(offset).map_err(corrupted)?;

                if let (Some(second), Ok(records)) = (
                    second_of(self.hour_start, entry.time),
                    decompress_records(entry.payload),
                ) {
                    result.0.add_records(second, &records);
                    result.1.add_records(second, &records);
                }

                offset += entry.len;
            }

            match frame.next() {
                Some(next) => frame_id = next,
                None => return Ok(result),
            }
        }

        Err(corrupted(CHAIN_LOOP))
    }

    /// Finds the cells again by walking the chain from its head. A frame, a link or an
    /// entry which does not read ends the chain there: the frame keeps its entries before
    /// it and becomes the last one. The walk reads every record, so it counts the key
    /// index and the process index too, and returns them: the ones on the disk are as
    /// stale as the cells were.
    fn rebuild(&mut self) -> std::io::Result<(KeyIndex, ProcessIndex)> {
        self.cells = HourCells::new();
        self.frames.clear();

        let mut key_index = KeyIndex::new();
        let mut process_index = ProcessIndex::new(self.hour_start);

        // A frame a file ends inside is cut off
        for data_file in &self.data_files {
            if let Some(file) = &data_file.file {
                file.set_len(data_file.frames_count as u64 * FRAME_LEN as u64)?;
            }
        }

        let mut visited = AHashSet::new();
        let mut frame_id = FrameId::HEAD;

        while self.frames_count() > 0 {
            visited.insert(frame_id);

            let mut frame = match self.read_frame_from_disk(frame_id) {
                Ok(frame) => frame,
                Err(_) => {
                    // Not even the head reads: there is nothing in the files
                    for data_file in &mut self.data_files {
                        if let Some(file) = &data_file.file {
                            file.set_len(0)?;
                        }

                        data_file.frames_count = 0;
                        data_file.frames_on_disk = 0;
                    }
                    break;
                }
            };

            let mut offset = FRAME_HEADER_LEN;
            let mut cut = false;

            while offset < frame.records_end() {
                let entry = frame.entry_at(offset).ok().and_then(|entry| {
                    second_of(self.hour_start, entry.time).map(|second| (second, entry))
                });

                let Some((second, entry)) = entry else {
                    cut = true;
                    break;
                };

                if self.cells.get(second).is_empty() {
                    self.cells.set(
                        second,
                        Cell {
                            frame: frame_id,
                            offset: offset as u32,
                        },
                    );
                }

                // The number of records is in the block: it is decompressed. One which
                // does not read stays where it is and counts for nothing.
                if let Ok(records) = decompress_records(entry.payload) {
                    self.cells
                        .set_count(second, self.cells.count(second) + records.len() as u32);
                    key_index.add_records(second, &records);
                    process_index.add_records(second, &records);
                }

                offset += entry.len;
            }

            let next = frame.next();
            let next_reads = next.map_or(false, |next| {
                !visited.contains(&next) && self.read_frame_from_disk(next).is_ok()
            });

            if cut || (next.is_some() && !next_reads) {
                if cut {
                    frame.split_off(offset);
                }

                frame.set_next(None);
                self.data_files[frame_id.file as usize]
                    .file
                    .as_ref()
                    .unwrap()
                    .write_all_at(frame.as_bytes(), frame_id.offset as u64)?;
                break;
            }

            match next {
                Some(next) => frame_id = next,
                None => break,
            }
        }

        let index_file = self.index_file.as_ref().unwrap();
        index_file.write_all_at(&self.cells.to_index_file(), 0)?;
        index_file.set_len(super::INDEX_FILE_LEN as u64)?;
        self.index_clean_on_disk = true;

        Ok((key_index, process_index))
    }

    /// Every entry of the chain in its order: its frame, offset and time
    #[cfg(test)]
    pub fn walk(&self) -> Vec<(FrameId, usize, i64)> {
        let mut result = Vec::new();

        if self.tail.is_none() {
            return result;
        }

        let mut frame_id = FrameId::HEAD;

        loop {
            let frame = self.read_frame(frame_id).unwrap();
            let mut offset = FRAME_HEADER_LEN;

            while offset < frame.records_end() {
                let entry = frame.entry_at(offset).unwrap();
                result.push((frame_id, offset, entry.time));
                offset += entry.len;
            }

            match frame.next() {
                Some(next) => frame_id = next,
                None => return result,
            }
        }
    }

    /// The records of every entry of the chain in its order
    #[cfg(test)]
    pub fn read_all(&self) -> Vec<Rec> {
        self.walk()
            .into_iter()
            .flat_map(|(frame_id, offset, _)| {
                let frame = self.read_frame(frame_id).unwrap();
                let entry = frame.entry_at(offset).unwrap();
                decompress_records(entry.payload).unwrap()
            })
            .collect()
    }

    /// The records of every entry of the chain whose block reads
    #[cfg(test)]
    pub fn read_all_which_read(&self) -> Vec<Rec> {
        self.walk()
            .into_iter()
            .flat_map(|(frame_id, offset, _)| {
                let frame = self.read_frame(frame_id).unwrap();
                let entry = frame.entry_at(offset).unwrap();
                decompress_records(entry.payload).unwrap_or_default()
            })
            .collect()
    }

    /// How many data files the hour has
    #[cfg(test)]
    pub fn data_files_count(&self) -> usize {
        self.data_files.len()
    }
}

fn corrupted(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}
