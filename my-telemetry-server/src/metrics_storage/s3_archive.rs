use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use my_s3::{S3Client, S3ListObjectsRequest};
use parking_lot::Mutex;
use rust_extensions::AsyncBytesStream;
use tokio::io::AsyncWriteExt;

use crate::{product_scope::is_valid_scope_name, settings::S3ConnectionSettings};

use super::{
    data_file_name, hour_key_of, hour_start_of_key, DataSource, DataStream, HourCells,
    KeyIndexDirectory, ProcessIndexDirectory, KEY_INDEX_FILE_NAME, PROCESS_INDEX_FILE_NAME,
    SECOND_INDEX_FILE_NAME,
};

/// A data file of an hour is gigabytes at most: what it is given to get to S3
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// A read of an object which gets no bytes for this long is given up: a search does not
/// hang on a connection which went silent
const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);

/// An hour found missing in S3 is asked for again after this
const MISSING_HOUR_TTL: Duration = Duration::from_secs(60);

/// Indexes of hours in S3 kept in memory
const INDEX_CACHE_SIZE: usize = 64;

/// What the bucket lists is asked for again after this
const LISTING_TTL: Duration = Duration::from_secs(60);

/// What the archive keeps its objects in: a bucket of S3 - or memory, in the tests
#[async_trait::async_trait]
pub trait ObjectStore: Send + Sync {
    /// A whole object; `None` - there is no such
    async fn download(&self, key: &str) -> Result<Option<Vec<u8>>, String>;

    /// An object from `offset` on, to `end` inclusive when it is given
    async fn open_stream(
        &self,
        key: &str,
        offset: u64,
        end: Option<u64>,
    ) -> Result<DataStream, String>;

    async fn upload(&self, key: &str, content: Vec<u8>) -> Result<(), String>;

    /// Streams the file, and checks the size it has there
    async fn upload_file(&self, key: &str, path: &Path) -> Result<(), String>;

    /// The size of an object; `None` - there is no such
    async fn size_of(&self, key: &str) -> Result<Option<u64>, String>;

    /// Every object whose key starts with the prefix, and its size
    async fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, String>;

    async fn delete(&self, key: &str) -> Result<(), String>;
}

/// An object of S3 - or a range of it - read as it comes
struct S3ObjectStream {
    // Held across the read of a chunk: `tokio::sync::Mutex`, not `parking_lot`
    stream: tokio::sync::Mutex<my_s3::S3DownloadStream>,
    size: Option<usize>,
}

impl S3ObjectStream {
    fn new(stream: my_s3::S3DownloadStream) -> Self {
        Self {
            size: stream.content_length.map(|len| len as usize),
            stream: tokio::sync::Mutex::new(stream),
        }
    }
}

#[async_trait::async_trait]
impl AsyncBytesStream<String> for S3ObjectStream {
    type Chunk = Vec<u8>;

    async fn get_next(&self) -> Result<Option<Vec<u8>>, String> {
        let mut stream = self.stream.lock().await;

        match tokio::time::timeout(CHUNK_TIMEOUT, stream.get_next_chunk()).await {
            Ok(chunk) => chunk.map_err(|err| err.to_string()),
            Err(_) => Err(format!(
                "No bytes from S3 for {} seconds",
                CHUNK_TIMEOUT.as_secs()
            )),
        }
    }

    fn get_size(&self) -> Option<usize> {
        self.size
    }
}

/// A bucket of S3
struct S3Bucket {
    client: S3Client,
    bucket: String,
    is_ready: AtomicBool,
}

impl S3Bucket {
    /// Before the first upload: the bucket is created when it is not there
    async fn make_ready(&self) -> Result<(), String> {
        if self.is_ready.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Created only when it is not there: keys allowed to write may not be allowed
        // to create a bucket
        match self.client.check_if_bucket_exists(&self.bucket).await {
            Ok(true) => {}
            Ok(false) => self
                .client
                .create_bucket_if_not_exists(&self.bucket)
                .await
                .map_err(|err| format!("Bucket {}: {}", self.bucket, err))?,
            Err(err) => println!(
                "Bucket {} is not checked: {}. The upload says whether it is there",
                self.bucket, err
            ),
        }

        self.is_ready.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[async_trait::async_trait]
impl ObjectStore for S3Bucket {
    async fn download(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match self.client.download_file(&self.bucket, key).await {
            Ok(stream) => Ok(Some(S3ObjectStream::new(stream).into_vec().await?)),
            Err(err) if err.is_key_not_found() => Ok(None),
            Err(err) => Err(format!("{}: {}", key, err)),
        }
    }

    async fn open_stream(
        &self,
        key: &str,
        offset: u64,
        end: Option<u64>,
    ) -> Result<DataStream, String> {
        let stream = self
            .client
            .download_file_range(&self.bucket, key, offset, end)
            .await
            .map_err(|err| format!("{}: {}", key, err))?;

        Ok(Arc::new(S3ObjectStream::new(stream)))
    }

    async fn upload(&self, key: &str, content: Vec<u8>) -> Result<(), String> {
        self.make_ready().await?;

        self.client
            .upload(&self.bucket, key, content, UPLOAD_TIMEOUT)
            .await
            .map_err(|err| format!("Upload of {}: {}", key, err))
    }

    async fn upload_file(&self, key: &str, path: &Path) -> Result<(), String> {
        self.make_ready().await?;

        let len = tokio::fs::metadata(path)
            .await
            .map_err(|err| format!("{}: {}", path.display(), err))?
            .len();

        let path = path.to_path_buf();

        self.client
            .upload_with_writer(
                &self.bucket,
                key,
                len as usize,
                UPLOAD_TIMEOUT,
                |mut writer| async move {
                    let mut file = tokio::fs::File::open(&path).await?;
                    tokio::io::copy(&mut file, &mut writer).await?;
                    writer.shutdown().await
                },
            )
            .await
            .map_err(|err| format!("Upload of {}: {}", key, err))?;

        let uploaded = self
            .client
            .get_object_size(&self.bucket, key)
            .await
            .map_err(|err| format!("{}: {}", key, err))?;

        if uploaded != len {
            return Err(format!(
                "{} is {} bytes in S3, and {} bytes were uploaded",
                key, uploaded, len
            ));
        }

        Ok(())
    }

    async fn size_of(&self, key: &str) -> Result<Option<u64>, String> {
        match self.client.get_object_size(&self.bucket, key).await {
            Ok(size) => Ok(Some(size)),
            Err(err) if err.is_key_not_found() => Ok(None),
            Err(err) => Err(format!("{}: {}", key, err)),
        }
    }

    async fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, String> {
        let mut result = Vec::new();
        let mut continuation_token: Option<String> = None;

        loop {
            let page = self
                .client
                .list_objects_v2(
                    &self.bucket,
                    S3ListObjectsRequest {
                        prefix: Some(prefix),
                        continuation_token: continuation_token.as_deref(),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|err| format!("Listing of {}: {}", self.bucket, err))?;

            result.extend(page.objects.into_iter().map(|object| (object.key, object.size)));

            continuation_token = page.next_continuation_token;

            if continuation_token.is_none() {
                return Ok(result);
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        self.client
            .delete_file(&self.bucket, key)
            .await
            .map_err(|err| format!("Delete of {}: {}", key, err))
    }
}

/// `trading/2026101112/second.index`: a file of an hour of a product scope in the
/// archive, past its prefix - the path it has under the folder of the metrics on the
/// local disk
pub fn archive_file_name(scope: &str, hour_start: i64, file: &str) -> String {
    format!("{}/{}/{}", scope, hour_key_of(hour_start), file)
}

/// The scope, the hour and the file a key of the archive names; `None` - the key is not
/// of a file of an hour
fn parse_key<'s>(prefix: &str, key: &'s str) -> Option<(&'s str, i64, &'s str)> {
    let mut parts = key.strip_prefix(prefix)?.splitn(3, '/');

    let scope = parts.next().filter(|scope| is_valid_scope_name(scope))?;

    let hour_start = parts
        .next()
        .filter(|hour| hour.len() == 10)
        .and_then(|hour| hour.parse::<i64>().ok())
        .and_then(hour_start_of_key)?;

    let file = parts.next().filter(|file| !file.is_empty() && !file.contains('/'))?;

    Some((scope, hour_start, file))
}

/// The keys of an hour in the order they are deleted: its index first, so an hour deleted
/// halfway is not one which is read
fn in_delete_order(mut keys: Vec<String>) -> Vec<String> {
    let is_index = |key: &String| key.ends_with(&format!("/{}", SECOND_INDEX_FILE_NAME));
    keys.sort_by_key(|key| !is_index(key));
    keys
}

/// The hours the archive has: by product scope and where the hour starts, how much the
/// hour takes. An hour is in the archive once its index is - it is uploaded the last.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ArchiveListing {
    hours: BTreeMap<(String, i64), u64>,
}

impl ArchiveListing {
    fn of(prefix: &str, objects: &[(String, u64)]) -> Self {
        let mut sizes: BTreeMap<(String, i64), u64> = BTreeMap::new();
        let mut indexed = BTreeSet::new();

        for (key, size) in objects {
            let Some((scope, hour_start, file)) = parse_key(prefix, key) else {
                continue;
            };

            *sizes.entry((scope.to_string(), hour_start)).or_default() += size;

            if file == SECOND_INDEX_FILE_NAME {
                indexed.insert((scope.to_string(), hour_start));
            }
        }

        sizes.retain(|hour, _| indexed.contains(hour));

        Self { hours: sizes }
    }

    pub fn scopes(&self) -> BTreeSet<&str> {
        self.hours.keys().map(|(scope, _)| scope.as_str()).collect()
    }

    /// The hours of the scope: where each starts and how much it takes
    pub fn hours_of<'s>(&'s self, scope: &'s str) -> impl Iterator<Item = (i64, u64)> + 's {
        self.hours
            .iter()
            .filter(move |((hour_scope, _), _)| hour_scope == scope)
            .map(|((_, hour_start), size)| (*hour_start, *size))
    }

    /// The scopes which have the hour
    pub fn scopes_of_hour(&self, hour_start: i64) -> impl Iterator<Item = &str> {
        self.hours
            .keys()
            .filter(move |(_, hour)| *hour == hour_start)
            .map(|(scope, _)| scope.as_str())
    }
}

/// The indexes of an hour in the archive: its cells, and the directories of its key index
/// and its process index
#[derive(Clone)]
pub struct ArchivedIndexes {
    pub cells: Arc<HourCells>,
    pub key_index: Arc<KeyIndexDirectory>,
    /// `None` - the hour has none, or it does not read: its traces scan it
    pub process_index: Option<Arc<ProcessIndexDirectory>>,
}

struct CachedIndexes {
    indexes: Option<ArchivedIndexes>,
    loaded: Instant,
}

/// The files of the copies of an hour which are uploaded
pub struct HourCopies {
    pub data: Vec<PathBuf>,
    pub second_index: PathBuf,
    pub key_index: PathBuf,
    pub process_index: PathBuf,
}

impl HourCopies {
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.data
            .iter()
            .chain([&self.second_index, &self.key_index, &self.process_index])
    }
}

/// The hours once they are uploaded: `{prefix}/{scope}/{hour}/` with the same files as
/// the folder of the hour on the local disk, their frames one after another.
pub struct S3Archive {
    store: Arc<dyn ObjectStore>,
    /// Ends with '/'
    prefix: String,
    /// By product scope and where the hour starts
    indexes: Mutex<BTreeMap<(String, i64), CachedIndexes>>,
    /// What the bucket listed the last time it was asked, and when: `None` - it did not
    /// answer
    listing: Mutex<Option<(Instant, Option<Arc<ArchiveListing>>)>>,
    /// Goes up with every change of what the archive has: a listing asked for before one
    /// is not kept after it
    changes: AtomicU64,
}

impl S3Archive {
    pub fn new(settings: &S3ConnectionSettings) -> Self {
        let client = S3Client::new(
            settings.access_key.as_str(),
            settings.secret_key.as_str(),
            settings.region.as_str(),
            settings.endpoint.as_str(),
        );

        // `Debug=1`: every request is traced to stdout - the verb, the url and the body
        // size going out, the whole answer body when it failed. The `Authorization`
        // header is not.
        let client = if settings.debug {
            println!("S3 request tracing is ON (Debug in S3ConnString)");
            client.debug_to_console()
        } else {
            client
        };

        Self::in_store(
            Arc::new(S3Bucket {
                client,
                bucket: settings.bucket.clone(),
                is_ready: AtomicBool::new(false),
            }),
            settings.prefix.as_str(),
        )
    }

    /// An archive under the prefix - a folder, with no slashes around it - of the store
    pub fn in_store(store: Arc<dyn ObjectStore>, prefix: &str) -> Self {
        Self {
            store,
            prefix: format!("{}/", prefix),
            indexes: Mutex::new(BTreeMap::new()),
            listing: Mutex::new(None),
            changes: AtomicU64::new(0),
        }
    }

    fn key(&self, file_name: &str) -> String {
        format!("{}{}", self.prefix, file_name)
    }

    /// A file of an hour - its name past the prefix - from `offset` on, to `end`
    /// inclusive when it is given
    pub async fn open_stream(
        &self,
        file_name: &str,
        offset: u64,
        end: Option<u64>,
    ) -> Result<DataStream, String> {
        self.store
            .open_stream(&self.key(file_name), offset, end)
            .await
    }

    fn source(archive: &Arc<S3Archive>, scope: &str, hour_start: i64, file: &str) -> DataSource {
        DataSource::S3 {
            archive: archive.clone(),
            file_name: archive_file_name(scope, hour_start, file),
        }
    }

    /// Where the key index of an hour of the archive is read from
    pub fn key_index_source(archive: &Arc<S3Archive>, scope: &str, hour_start: i64) -> DataSource {
        Self::source(archive, scope, hour_start, KEY_INDEX_FILE_NAME)
    }

    /// Where the process index of an hour of the archive is read from
    pub fn process_index_source(
        archive: &Arc<S3Archive>,
        scope: &str,
        hour_start: i64,
    ) -> DataSource {
        Self::source(archive, scope, hour_start, PROCESS_INDEX_FILE_NAME)
    }

    /// The indexes of an hour of the scope; `None` - the hour is not in the archive
    pub async fn get_indexes(
        archive: &Arc<S3Archive>,
        scope: &str,
        hour_start: i64,
    ) -> Result<Option<ArchivedIndexes>, String> {
        let this = archive.as_ref();
        let cache_key = (scope.to_string(), hour_start);

        if let Some(cached) = this.indexes.lock().get(&cache_key) {
            if cached.indexes.is_some() || cached.loaded.elapsed() < MISSING_HOUR_TTL {
                return Ok(cached.indexes.clone());
            }
        }

        let name = format!("{}/{}", scope, hour_key_of(hour_start));

        let index_key = this.key(&archive_file_name(scope, hour_start, SECOND_INDEX_FILE_NAME));

        // An index which did not read this time is asked for again the next time: what
        // is given without it is not kept, or the hour would be scanned by every search
        // for as long as it is in memory
        let mut is_whole = true;

        let indexes_of_hour = match this.store.download(&index_key).await? {
            Some(bytes) => {
                let cells = HourCells::from_index_file(&bytes)
                    .ok_or_else(|| format!("The index of {} in S3 is not an index", name))?;

                // Without a key index every search of the hour scans it
                let key_index =
                    match KeyIndexDirectory::read(&Self::key_index_source(archive, scope, hour_start))
                        .await
                    {
                        Ok(directory) => directory,
                        Err(err) => {
                            println!("The key index of {} in S3: {}", name, err);
                            is_whole = false;
                            KeyIndexDirectory::empty()
                        }
                    };

                // And without a process index every trace of it does
                let process_index = match ProcessIndexDirectory::read(
                    hour_start,
                    &Self::process_index_source(archive, scope, hour_start),
                )
                .await
                {
                    Ok(directory) => Some(Arc::new(directory)),
                    Err(err) => {
                        println!("The process index of {} in S3: {}", name, err);
                        is_whole = false;
                        None
                    }
                };

                Some(ArchivedIndexes {
                    cells: Arc::new(cells),
                    key_index: Arc::new(key_index),
                    process_index,
                })
            }
            None => None,
        };

        if !is_whole {
            return Ok(indexes_of_hour);
        }

        let mut indexes = this.indexes.lock();

        if indexes.len() >= INDEX_CACHE_SIZE {
            let oldest = indexes
                .iter()
                .min_by_key(|(_, cached)| cached.loaded)
                .map(|(key, _)| key.clone());

            if let Some(oldest) = oldest {
                indexes.remove(&oldest);
            }
        }

        indexes.insert(
            cache_key,
            CachedIndexes {
                indexes: indexes_of_hour.clone(),
                loaded: Instant::now(),
            },
        );

        Ok(indexes_of_hour)
    }

    /// The hour is in the archive: its index is - what an upload ends with
    pub async fn has_hour(&self, scope: &str, hour_start: i64) -> Result<bool, String> {
        let index = self.key(&archive_file_name(scope, hour_start, SECOND_INDEX_FILE_NAME));
        Ok(self.store.size_of(&index).await?.is_some())
    }

    /// Uploads the data files - `copies.data[n]` as `metrics.dataNN` - checking the size
    /// of each, then the key index, the process index and the index: an hour is in the
    /// archive once its index is.
    ///
    /// A data file which is there already, as long as the copy, is not uploaded again: an
    /// upload which did not complete goes on from where it stopped. The copy of an hour is
    /// the same each time it is made - nothing is written into an hour which is uploaded.
    pub async fn upload(
        &self,
        scope: &str,
        hour_start: i64,
        copies: &HourCopies,
    ) -> Result<(), String> {
        for (file, path) in copies.data.iter().enumerate() {
            let key = self.key(&archive_file_name(scope, hour_start, &data_file_name(file as u8)));

            let len = tokio::fs::metadata(path)
                .await
                .map_err(|err| format!("{}: {}", path.display(), err))?
                .len();

            if self.store.size_of(&key).await? != Some(len) {
                self.store.upload_file(&key, path).await?;
            }
        }

        for (path, file) in [
            (&copies.key_index, KEY_INDEX_FILE_NAME),
            (&copies.process_index, PROCESS_INDEX_FILE_NAME),
            (&copies.second_index, SECOND_INDEX_FILE_NAME),
        ] {
            let content = tokio::fs::read(path)
                .await
                .map_err(|err| format!("{}: {}", path.display(), err))?;

            self.store
                .upload(&self.key(&archive_file_name(scope, hour_start, file)), content)
                .await?;
        }

        self.forget(scope, hour_start);

        Ok(())
    }

    /// What is kept in memory of the hour is not what the archive has any more
    fn forget(&self, scope: &str, hour_start: i64) {
        self.changes.fetch_add(1, Ordering::Relaxed);
        self.indexes.lock().remove(&(scope.to_string(), hour_start));
        *self.listing.lock() = None;
    }

    /// The hours the archive has, as the bucket listed them a minute ago at most. `None` -
    /// it does not answer: it is asked again in a minute, not by every request which
    /// lists the hours meanwhile.
    pub async fn listing(&self) -> Option<Arc<ArchiveListing>> {
        if let Some((asked, listing)) = &*self.listing.lock() {
            if asked.elapsed() < LISTING_TTL {
                return listing.clone();
            }
        }

        let changes = self.changes.load(Ordering::Relaxed);

        let listing = match self.store.list(&self.prefix).await {
            Ok(objects) => Some(Arc::new(ArchiveListing::of(&self.prefix, &objects))),
            Err(err) => {
                println!("The hours of S3 are not listed: {}", err);
                None
            }
        };

        // An hour uploaded or deleted while the bucket was listing: the answer may be
        // without it. It is given - and asked for again by whoever comes next.
        if changes == self.changes.load(Ordering::Relaxed) {
            *self.listing.lock() = Some((Instant::now(), listing.clone()));
        }

        listing
    }

    /// Deletes every hour which starts at `cutoff` or before - every object of it the
    /// bucket lists: its index first, so an hour deleted halfway is not read. Returns the
    /// scope and the start of each.
    pub async fn delete_up_to(&self, cutoff: i64) -> Result<Vec<(String, i64)>, String> {
        let mut hours: BTreeMap<(String, i64), Vec<String>> = BTreeMap::new();

        for (key, _) in self.store.list(&self.prefix).await? {
            if let Some((scope, hour_start, _)) = parse_key(&self.prefix, &key) {
                if hour_start <= cutoff {
                    hours
                        .entry((scope.to_string(), hour_start))
                        .or_default()
                        .push(key);
                }
            }
        }

        for ((scope, hour_start), keys) in &mut hours {
            for key in in_delete_order(std::mem::take(keys)) {
                self.store.delete(&key).await?;
            }

            self.forget(scope, *hour_start);
        }

        Ok(hours.into_keys().collect())
    }
}

/// An object store in memory
#[cfg(test)]
pub mod memory {
    use std::collections::VecDeque;

    use super::*;

    /// How much of an object one chunk of its stream is: little, so a read of a frame
    /// takes more than one
    const CHUNK_LEN: usize = 10_000;

    struct MemoryStream {
        chunks: Mutex<VecDeque<Vec<u8>>>,
        size: usize,
    }

    #[async_trait::async_trait]
    impl AsyncBytesStream<String> for MemoryStream {
        type Chunk = Vec<u8>;

        async fn get_next(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(self.chunks.lock().pop_front())
        }

        fn get_size(&self) -> Option<usize> {
            Some(self.size)
        }
    }

    #[derive(Default)]
    pub struct MemoryStore {
        pub objects: Mutex<BTreeMap<String, Vec<u8>>>,
        /// Uploads fail while it is set
        pub is_down: AtomicBool,
        /// Uploads of the keys which end with this fail
        pub refuses: Mutex<Option<String>>,
        /// The keys uploaded, in the order they were
        pub uploaded: Mutex<Vec<String>>,
        /// How many bytes the streams which were opened were asked for
        pub bytes_asked: Mutex<Vec<(String, u64)>>,
    }

    impl MemoryStore {
        pub fn keys(&self) -> Vec<String> {
            self.objects.lock().keys().cloned().collect()
        }

        fn take_upload(&self, key: &str, content: Vec<u8>) -> Result<(), String> {
            if self.is_down.load(Ordering::Relaxed) {
                return Err("The store is down".to_string());
            }

            if let Some(refused) = &*self.refuses.lock() {
                if key.ends_with(refused.as_str()) {
                    return Err(format!("{} is refused", key));
                }
            }

            self.uploaded.lock().push(key.to_string());
            self.objects.lock().insert(key.to_string(), content);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore for MemoryStore {
        async fn download(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.objects.lock().get(key).cloned())
        }

        async fn open_stream(
            &self,
            key: &str,
            offset: u64,
            end: Option<u64>,
        ) -> Result<DataStream, String> {
            let objects = self.objects.lock();

            let object = objects.get(key).ok_or_else(|| format!("{}: not found", key))?;

            let from = offset as usize;
            let to = end.map_or(object.len(), |end| (end as usize + 1).min(object.len()));

            if from >= object.len() {
                return Err(format!("{}: the range is past its end", key));
            }

            self.bytes_asked
                .lock()
                .push((key.to_string(), (to - from) as u64));

            let range = &object[from..to];

            Ok(Arc::new(MemoryStream {
                size: range.len(),
                chunks: Mutex::new(range.chunks(CHUNK_LEN).map(<[u8]>::to_vec).collect()),
            }))
        }

        async fn upload(&self, key: &str, content: Vec<u8>) -> Result<(), String> {
            self.take_upload(key, content)
        }

        async fn upload_file(&self, key: &str, path: &Path) -> Result<(), String> {
            let content = std::fs::read(path).map_err(|err| format!("{}: {}", path.display(), err))?;
            self.take_upload(key, content)
        }

        async fn size_of(&self, key: &str) -> Result<Option<u64>, String> {
            Ok(self.objects.lock().get(key).map(|content| content.len() as u64))
        }

        async fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, String> {
            Ok(self
                .objects
                .lock()
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, content)| (key.clone(), content.len() as u64))
                .collect())
        }

        async fn delete(&self, key: &str) -> Result<(), String> {
            self.objects.lock().remove(key);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{memory::MemoryStore, *};
    use crate::metrics_storage::{HOUR_MICROS, MICROS_PER_SECOND};

    /// 2023-11-14 22:00:00 UTC
    const HOUR: i64 = 1_699_999_200 * MICROS_PER_SECOND;

    #[test]
    fn a_key_names_the_scope_the_hour_and_the_file() {
        assert_eq!(
            archive_file_name("trading", HOUR, "metrics.data0A"),
            "trading/2023111422/metrics.data0A"
        );

        assert_eq!(
            parse_key("prod-1/", "prod-1/trading/2023111422/metrics.data0A"),
            Some(("trading", HOUR, "metrics.data0A"))
        );
        assert_eq!(
            parse_key("telemetry/prod/", "telemetry/prod/default/2023111423/second.index"),
            Some(("default", HOUR + HOUR_MICROS, "second.index"))
        );

        // The folder of another server, a name which is not a scope, not an hour, not a
        // file of one
        assert_eq!(parse_key("prod-1/", "prod-2/trading/2023111422/second.index"), None);
        assert_eq!(parse_key("prod-1/", "prod-1/My Scope/2023111422/second.index"), None);
        assert_eq!(parse_key("prod-1/", "prod-1/trading/2023111424/second.index"), None);
        assert_eq!(parse_key("prod-1/", "prod-1/trading/202311142/second.index"), None);
        assert_eq!(parse_key("prod-1/", "prod-1/trading/2023111422/"), None);
        assert_eq!(parse_key("prod-1/", "prod-1/trading/2023111422"), None);
        assert_eq!(parse_key("prod-1/", "prod-1/trading/2023111422/a/second.index"), None);
    }

    #[test]
    fn an_hour_is_listed_once_its_index_is_there() {
        let objects = [
            ("p/default/2023111422/metrics.data00".to_string(), 1000),
            ("p/default/2023111422/metrics.data01".to_string(), 500),
            ("p/default/2023111422/key.index".to_string(), 30),
            ("p/default/2023111422/process.index".to_string(), 20),
            ("p/default/2023111422/second.index".to_string(), 10),
            // Its upload is not over, or it is deleted halfway: not an hour to read
            ("p/default/2023111423/metrics.data00".to_string(), 7000),
            ("p/default/2023111423/key.index".to_string(), 70),
            ("p/trading/2023111423/metrics.data00".to_string(), 200),
            ("p/trading/2023111423/second.index".to_string(), 10),
            // Not of the archive
            ("p/readme.txt".to_string(), 5),
            ("other/default/2023111422/second.index".to_string(), 10),
        ];

        let listing = ArchiveListing::of("p/", &objects);

        assert_eq!(listing.scopes().into_iter().collect::<Vec<_>>(), ["default", "trading"]);
        assert_eq!(listing.hours_of("default").collect::<Vec<_>>(), [(HOUR, 1560)]);
        assert_eq!(
            listing.hours_of("trading").collect::<Vec<_>>(),
            [(HOUR + HOUR_MICROS, 210)]
        );
        assert_eq!(listing.hours_of("prop").count(), 0);

        assert_eq!(listing.scopes_of_hour(HOUR).collect::<Vec<_>>(), ["default"]);
        assert_eq!(
            listing.scopes_of_hour(HOUR + HOUR_MICROS).collect::<Vec<_>>(),
            ["trading"]
        );
    }

    #[test]
    fn the_index_of_an_hour_is_deleted_first() {
        let keys = vec![
            "p/default/2023111422/key.index".to_string(),
            "p/default/2023111422/metrics.data00".to_string(),
            "p/default/2023111422/process.index".to_string(),
            "p/default/2023111422/second.index".to_string(),
        ];

        let ordered = in_delete_order(keys);

        assert_eq!(ordered[0], "p/default/2023111422/second.index");
        assert_eq!(ordered.len(), 4);
    }

    #[tokio::test]
    async fn hours_are_deleted_up_to_the_cutoff() {
        let store = Arc::new(MemoryStore::default());
        let archive = S3Archive::in_store(store.clone(), "p");

        for (scope, hour_start) in [
            ("default", HOUR),
            ("default", HOUR + HOUR_MICROS),
            ("trading", HOUR),
            ("trading", HOUR + 2 * HOUR_MICROS),
        ] {
            for file in ["metrics.data00", "key.index", "process.index", "second.index"] {
                let key = format!("p/{}", archive_file_name(scope, hour_start, file));
                store.upload(&key, vec![1, 2, 3]).await.unwrap();
            }
        }

        // The folder of another server in the same bucket is not touched
        store
            .upload("p2/default/2023111422/second.index", vec![1])
            .await
            .unwrap();

        assert_eq!(archive.listing().await.unwrap().hours_of("default").count(), 2);

        // The folder of another server: nothing of it is listed here
        let other = S3Archive::in_store(store.clone(), "p2");
        assert_eq!(other.listing().await.unwrap().hours_of("default").count(), 1);
        assert_eq!(other.listing().await.unwrap().hours_of("trading").count(), 0);
        assert!(archive.has_hour("trading", HOUR).await.unwrap());

        let deleted = archive.delete_up_to(HOUR + HOUR_MICROS).await.unwrap();

        assert_eq!(
            deleted,
            [
                ("default".to_string(), HOUR),
                ("default".to_string(), HOUR + HOUR_MICROS),
                ("trading".to_string(), HOUR),
            ]
        );

        assert!(!archive.has_hour("trading", HOUR).await.unwrap());
        assert!(archive.has_hour("trading", HOUR + 2 * HOUR_MICROS).await.unwrap());

        // The listing is not the one of a minute ago
        let listing = archive.listing().await.unwrap();
        assert_eq!(listing.hours_of("default").count(), 0);
        assert_eq!(
            listing.hours_of("trading").collect::<Vec<_>>(),
            [(HOUR + 2 * HOUR_MICROS, 12)]
        );

        assert_eq!(
            store.keys().iter().filter(|key| key.starts_with("p2/")).count(),
            1
        );
    }
}
