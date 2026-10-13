use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rust_extensions::{AsyncBytesStream, BufferedReader};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::{
    archive_file_name, data_file_name, Entry, Frame, FrameId, HourCells, S3Archive,
    FRAME_HEADER_LEN, FRAME_LEN, MAX_DATA_FILES, MICROS_PER_SECOND,
};

/// How much of a local file one chunk of its stream is - and how much a read of a part of
/// a file asks for at least
pub const READ_LEN: usize = 64 * 1024;

/// A chain longer than this many frames returns to a frame it walked: the data files of an
/// hour hold no more
const MAX_HOPS: usize = MAX_DATA_FILES * 131_072;

pub type DataStream = Arc<dyn AsyncBytesStream<String, Chunk = Vec<u8>> + Send + Sync>;

/// A local file read from an offset on. `FileStreamReader` of rust-extensions reads a
/// file from its start.
pub struct LocalFileStream {
    file: tokio::sync::Mutex<tokio::fs::File>,
    size: usize,
}

impl LocalFileStream {
    pub async fn open(path: &Path, offset: u64) -> std::io::Result<Self> {
        let mut file = tokio::fs::File::open(path).await?;
        let len = file.metadata().await?.len();
        file.seek(std::io::SeekFrom::Start(offset)).await?;

        Ok(Self {
            file: tokio::sync::Mutex::new(file),
            size: len.saturating_sub(offset) as usize,
        })
    }
}

#[async_trait::async_trait]
impl AsyncBytesStream<String> for LocalFileStream {
    type Chunk = Vec<u8>;

    async fn get_next(&self) -> Result<Option<Vec<u8>>, String> {
        // The guard is held across the read: `tokio::sync::Mutex`, not `parking_lot`
        let mut file = self.file.lock().await;

        let mut chunk = vec![0u8; READ_LEN];
        let read = file.read(&mut chunk).await.map_err(|err| err.to_string())?;

        if read == 0 {
            return Ok(None);
        }

        chunk.truncate(read);
        Ok(Some(chunk))
    }

    fn get_size(&self) -> Option<usize> {
        Some(self.size)
    }
}

/// Where a file of an hour is read from
#[derive(Clone)]
pub enum DataSource {
    Local(PathBuf),
    S3 {
        archive: Arc<S3Archive>,
        /// Past the prefix of the archive
        file_name: String,
    },
}

impl DataSource {
    /// The file from `offset` on: to `end` inclusive when it is given - S3 is asked for
    /// that range only
    async fn open(&self, offset: u64, end: Option<u64>) -> Result<DataStream, String> {
        match self {
            DataSource::Local(path) => {
                let stream = LocalFileStream::open(path, offset)
                    .await
                    .map_err(|err| format!("{}: {}", path.display(), err))?;

                Ok(Arc::new(stream))
            }
            DataSource::S3 { archive, file_name } => {
                archive.open_stream(file_name, offset, end).await
            }
        }
    }
}

/// `len` bytes of the file from `offset` on - fewer where the file ends - read through a
/// `BufferedReader`
pub async fn read_bytes(source: &DataSource, offset: u64, len: usize) -> Result<Vec<u8>, String> {
    if len == 0 {
        return Ok(Vec::new());
    }

    let stream = source.open(offset, Some(offset + len as u64 - 1)).await?;
    let mut reader = BufferedReader::new(stream);

    while reader.as_slice().len() < len && reader.read_mode() {
        reader.get_next().await?;
    }

    let read = reader.as_slice().len().min(len);
    Ok(reader.as_slice()[..read].to_vec())
}

/// The data files of an hour: `metrics.data00`, `metrics.data01`... in the folder of the
/// hour on the local disk, or in S3
#[derive(Clone)]
pub enum DataFiles {
    Local {
        dir: PathBuf,
    },
    S3 {
        archive: Arc<S3Archive>,
        scope: Arc<str>,
        hour_start: i64,
    },
}

impl DataFiles {
    pub fn file(&self, file: u8) -> DataSource {
        match self {
            DataFiles::Local { dir } => DataSource::Local(dir.join(data_file_name(file))),
            DataFiles::S3 {
                archive,
                scope,
                hour_start,
            } => DataSource::S3 {
                archive: archive.clone(),
                file_name: archive_file_name(scope, *hour_start, &data_file_name(file)),
            },
        }
    }
}

/// Reads the frames of a chain through a `BufferedReader`: from one stream while each
/// frame is the one right after the one before in the same file, from a new one where the
/// chain jumps or goes over to another file.
struct FramesReader {
    files: DataFiles,
    /// Where the frames to read end in a file: S3 is asked for that range only
    end: Option<FrameId>,
    reader: Option<BufferedReader<String, DataStream>>,
    position: FrameId,
}

impl FramesReader {
    fn new(files: DataFiles, end: Option<FrameId>) -> Self {
        Self {
            files,
            end,
            reader: None,
            position: FrameId::HEAD,
        }
    }

    async fn read_frame(&mut self, frame: FrameId) -> Result<Frame, String> {
        if self.reader.is_none() || self.position != frame {
            self.open_at(frame).await?;
        }

        loop {
            let reader = self.reader.as_mut().unwrap();

            if reader.as_slice().len() >= FRAME_LEN {
                let read = Frame::from_bytes(reader.as_slice()[..FRAME_LEN].to_vec())?;
                reader.mark_as_read(FRAME_LEN);
                self.position.offset += FRAME_LEN as u32;
                return Ok(read);
            }

            if reader.read_mode() {
                reader.get_next().await?;
                continue;
            }

            // The range asked for is over: the rest of the file is read
            if self.end.is_some() {
                self.end = None;
                self.open_at(frame).await?;
                continue;
            }

            return Err(format!("A data file ends inside the frame {:?}", frame));
        }
    }

    async fn open_at(&mut self, frame: FrameId) -> Result<(), String> {
        let end = self
            .end
            .filter(|end| end.file == frame.file && end.offset >= frame.offset)
            .map(|end| end.offset as u64 + FRAME_LEN as u64 - 1);

        let stream = self
            .files
            .file(frame.file)
            .open(frame.offset as u64, end)
            .await?;

        self.reader = Some(BufferedReader::new(stream));
        self.position = frame;

        Ok(())
    }
}

/// Walks the entries of the seconds `from_second ..= to_second` of an hour in the order of
/// its chain: from the first entry of the first of them which has one, to the first entry
/// of a later second - or until `on_entry` says `false`.
pub async fn scan_seconds(
    files: DataFiles,
    cells: &HourCells,
    hour_start: i64,
    from_second: usize,
    to_second: usize,
    mut on_entry: impl FnMut(&Entry) -> bool,
) -> Result<(), String> {
    let Some((first_second, start)) = cells.first_from(from_second) else {
        return Ok(());
    };

    if first_second > to_second {
        return Ok(());
    }

    // The entries of the window end in the frame where the next second starts
    let end = cells.first_from(to_second + 1).map(|(_, cell)| cell.frame);

    let to_exclusive = hour_start + (to_second as i64 + 1) * MICROS_PER_SECOND;

    let mut frames = FramesReader::new(files, end);
    let mut frame_id = start.frame;
    let mut offset = start.offset as usize;

    for _ in 0..MAX_HOPS {
        let frame = frames.read_frame(frame_id).await?;

        while offset < frame.records_end() {
            let entry = frame.entry_at(offset)?;

            if entry.time >= to_exclusive || !on_entry(&entry) {
                return Ok(());
            }

            offset += entry.len;
        }

        let Some(next) = frame.next() else {
            return Ok(());
        };

        frame_id = next;
        offset = FRAME_HEADER_LEN;
    }

    Err("The chain of frames returns to a frame it walked".to_string())
}
