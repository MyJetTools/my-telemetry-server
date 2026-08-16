use rust_extensions::date_time::{HourKey, IntervalKey};
use tokio::sync::Mutex;

use crate::db::MetricDto;

use super::second_within_hour;
use super::storage_by_hour_inner::StorageByHourInner;

/// How many events a "last events" screen asks for. Kept here rather than at the call site
/// because it bounds what a scan has to hold.
const LAST_EVENTS_LIMIT: usize = 100;

/// Saving and searching metrics for one hour.
///
/// The wrapper owns concurrency and nothing else - every decision lives in
/// [`StorageByHourInner`].
pub struct StorageByHour {
    /// `tokio::sync::Mutex` rather than `parking_lot`, deliberately: the critical section
    /// spans file I/O. A heavy insert rewrites the data tail and then the index, and a
    /// reader that got in between the two would read offsets that no longer point at the
    /// records they name. The guard has to survive `.await`, which `parking_lot` cannot do.
    inner: Mutex<StorageByHourInner>,
}

impl StorageByHour {
    pub fn new(hour_key: IntervalKey<HourKey>, db_path: &str) -> Self {
        Self {
            inner: Mutex::new(StorageByHourInner::new(hour_key, db_path)),
        }
    }

    /// Takes metrics into the in-memory window. No record reaches disk here.
    ///
    /// The hour's folder is created on the first push even though it stays empty until the
    /// first flush: the list of available hours is built by scanning the folder, so
    /// otherwise the current hour would be missing from the UI for a whole window after
    /// every hour boundary.
    pub async fn push_range(&self, items: Vec<MetricDto>) {
        let mut write_access = self.inner.lock().await;

        write_access.ensure_created().await;

        for dto in items {
            write_access.push(dto);
        }
    }

    /// Moves everything older than `flush_before` (unix microseconds) onto disk.
    pub async fn flush(&self, flush_before: i64) {
        self.inner.lock().await.flush(flush_before).await;
    }

    /// Empties the window onto disk regardless of age - the hour is closing.
    pub async fn flush_all(&self) {
        self.inner.lock().await.flush_all().await;
    }

    /// Every event of one process. `ProcessId` is a correlation id, so this is the trace
    /// behind a single screen click.
    ///
    /// The second index does not serve this axis, so it is a scan of the hour. Ordered by
    /// `started` ascending, which is the order a trace reads in.
    pub async fn get_by_process_id(&self, process_id: i64) -> Vec<MetricDto> {
        self.inner
            .lock()
            .await
            .scan(0, None, |dto| dto.id == process_id)
            .await
    }

    /// The last events of one service action, newest first.
    ///
    /// `started` is a lower bound - it is what lets the second index skip straight to the
    /// right place instead of scanning from the top of the hour.
    pub async fn get_by_service_name(
        &self,
        service_name: &str,
        data: &str,
        client_id: Option<&str>,
        started: Option<i64>,
    ) -> Vec<MetricDto> {
        let from_second = match started {
            Some(started) => second_within_hour(started),
            None => 0,
        };

        let mut result = self
            .inner
            .lock()
            .await
            .scan(from_second, Some(LAST_EVENTS_LIMIT), |dto| {
                if dto.name != service_name || dto.data != data {
                    return false;
                }

                if let Some(client_id) = client_id {
                    if dto.client_id.as_deref() != Some(client_id) {
                        return false;
                    }
                }

                if let Some(started) = started {
                    if dto.started < started {
                        return false;
                    }
                }

                true
            })
            .await;

        // The scan hands back ascending order; the screen wants the newest.
        result.reverse();
        result.truncate(LAST_EVENTS_LIMIT);

        result
    }

    /// Events still in the window, i.e. accepted but not yet on disk. Surfaced by tech
    /// metrics, and it is also what a crash would cost.
    pub async fn window_len(&self) -> usize {
        self.inner.lock().await.window_len()
    }

    /// Releases the files and hands back the folder that holds them, so the caller can
    /// delete the hour whole.
    pub async fn close(&self) -> String {
        let mut write_access = self.inner.lock().await;

        let folder = write_access.folder().to_string();
        write_access.close();

        folder
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rust_extensions::date_time::DateTimeAsMicroseconds;

    use super::*;
    use crate::db::MetricDto;

    /// 2023-11-15 04:00:00 UTC - exactly on an hour boundary, so "second within hour"
    /// in the tests is the same number the index slots use.
    const HOUR_START_SECONDS: i64 = 1_699_999_200;
    const HOUR_START_MICROS: i64 = HOUR_START_SECONDS * 1_000_000;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

            let path = std::env::temp_dir().join(format!(
                "my-telemetry-storage-test-{}-{}",
                std::process::id(),
                unique
            ));

            std::fs::create_dir_all(&path).unwrap();

            Self { path }
        }

        fn db_path(&self) -> String {
            self.path.to_str().unwrap().to_string()
        }

        fn data_file(&self) -> String {
            self.hour_folder()
                .join(super::super::DATA_FILE_NAME)
                .to_str()
                .unwrap()
                .to_string()
        }

        fn index_file(&self) -> String {
            self.hour_folder()
                .join(super::super::SECOND_INDEX_FILE_NAME)
                .to_str()
                .unwrap()
                .to_string()
        }

        fn hour_folder(&self) -> std::path::PathBuf {
            self.path.join(hour_key().to_i64().to_string())
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    async fn push_one(storage: &StorageByHour, dto: MetricDto) {
        storage.push_range(vec![dto]).await;
    }

    fn hour_key() -> IntervalKey<HourKey> {
        DateTimeAsMicroseconds::new(HOUR_START_MICROS).into()
    }

    fn at_second(second: i64) -> i64 {
        HOUR_START_MICROS + second * 1_000_000
    }

    fn metric(second: i64, process_id: i64, data: &str) -> MetricDto {
        MetricDto {
            id: process_id,
            started: at_second(second),
            duration_micro: 100,
            name: "my-service".to_string(),
            data: data.to_string(),
            success: Some("ok".to_string()),
            fail: None,
            tags: None,
            client_id: Some("client-1".to_string()),
        }
    }

    /// Seconds of everything stored for `process_id`, in the order the scan yields them -
    /// which is the order the records physically sit in the file.
    async fn stored_seconds(storage: &StorageByHour, process_id: i64) -> Vec<i64> {
        storage
            .get_by_process_id(process_id)
            .await
            .into_iter()
            .map(|dto| (dto.started - HOUR_START_MICROS) / 1_000_000)
            .collect()
    }

    #[tokio::test]
    async fn an_hour_is_a_folder_named_by_its_key() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        push_one(&storage, metric(10, 1, "a")).await;
        storage.flush_all().await;

        let folder = dir.hour_folder();

        // 2023111504 - the key and nothing else, so the name is the key.
        assert_eq!(
            folder.file_name().unwrap().to_str().unwrap(),
            hour_key().to_i64().to_string()
        );

        assert!(folder.is_dir());
        assert!(folder.join(super::super::DATA_FILE_NAME).is_file());
        assert!(folder.join(super::super::SECOND_INDEX_FILE_NAME).is_file());

        // Deleting the hour is deleting one folder - there is no way to leave half of it.
        assert_eq!(storage.close().await, folder.to_str().unwrap());
    }

    #[tokio::test]
    async fn the_hour_exists_on_disk_before_anything_is_flushed() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        push_one(&storage, metric(10, 1, "a")).await;

        // Nothing has been flushed - but the folder scan that builds the list of available
        // hours has to find this hour already, or the UI loses the current hour for a
        // whole window after every boundary.
        assert!(dir.hour_folder().is_dir());
        assert_eq!(storage.window_len().await, 1);
    }

    #[tokio::test]
    async fn window_is_searchable_before_it_reaches_disk() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        push_one(&storage, metric(10, 1, "GET /orders")).await;

        // Nothing has been flushed - if the window were not merged into the read path,
        // the UI would not see what just happened.
        assert_eq!(storage.window_len().await, 1);
        assert_eq!(stored_seconds(&storage, 1).await, vec![10]);
    }

    #[tokio::test]
    async fn flush_moves_only_what_is_older_than_the_boundary() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        push_one(&storage, metric(10, 1, "a")).await;
        push_one(&storage, metric(20, 1, "b")).await;
        push_one(&storage, metric(30, 1, "c")).await;

        storage.flush(at_second(25)).await;

        // 10 and 20 landed, 30 is still in the window - and all three are still findable.
        assert_eq!(storage.window_len().await, 1);
        assert_eq!(stored_seconds(&storage, 1).await, vec![10, 20, 30]);
    }

    #[tokio::test]
    async fn out_of_order_within_the_window_never_touches_the_disk_out_of_order() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        // Arrives late, but still inside the window - absorbed by the in-memory sort.
        push_one(&storage, metric(30, 1, "c")).await;
        push_one(&storage, metric(10, 1, "a")).await;
        push_one(&storage, metric(20, 1, "b")).await;

        storage.flush_all().await;
        assert_eq!(storage.window_len().await, 0);

        assert_eq!(stored_seconds(&storage, 1).await, vec![10, 20, 30]);
    }

    #[tokio::test]
    async fn survives_a_reopen_from_its_own_files() {
        let dir = TestDir::new();

        {
            let storage = StorageByHour::new(hour_key(), &dir.db_path());
            push_one(&storage, metric(10, 7, "a")).await;
            push_one(&storage, metric(20, 7, "b")).await;
            storage.flush_all().await;
        }

        let reopened = StorageByHour::new(hour_key(), &dir.db_path());

        assert_eq!(stored_seconds(&reopened, 7).await, vec![10, 20]);
    }

    #[tokio::test]
    async fn late_beyond_the_window_is_inserted_in_its_place() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        push_one(&storage, metric(10, 1, "a")).await;
        push_one(&storage, metric(20, 1, "b")).await;
        push_one(&storage, metric(30, 1, "c")).await;
        storage.flush_all().await;

        // Second 15 is now older than everything on disk - the heavy path.
        push_one(&storage, metric(15, 1, "late")).await;
        storage.flush_all().await;

        assert_eq!(stored_seconds(&storage, 1).await, vec![10, 15, 20, 30]);

        // Reopening rebuilds nothing from memory, so this proves the bytes on disk are
        // ordered and the index still points at the right places.
        let reopened = StorageByHour::new(hour_key(), &dir.db_path());
        assert_eq!(stored_seconds(&reopened, 1).await, vec![10, 15, 20, 30]);
    }

    #[tokio::test]
    async fn late_record_into_a_second_that_already_has_data() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        push_one(&storage, metric(10, 1, "a")).await;
        push_one(&storage, metric(20, 1, "b")).await;
        storage.flush_all().await;

        // Same second as an existing record, and later within it.
        let mut late = metric(10, 1, "same-second");
        late.started += 500_000;
        push_one(&storage, late).await;
        storage.flush_all().await;

        let reopened = StorageByHour::new(hour_key(), &dir.db_path());
        let found = reopened.get_by_process_id(1).await;

        let order: Vec<&str> = found.iter().map(|dto| dto.data.as_str()).collect();
        assert_eq!(order, vec!["a", "same-second", "b"]);
    }

    #[tokio::test]
    async fn a_lost_index_is_rebuilt_from_the_data_file() {
        let dir = TestDir::new();

        {
            let storage = StorageByHour::new(hour_key(), &dir.db_path());
            push_one(&storage, metric(10, 3, "a")).await;
            push_one(&storage, metric(2000, 3, "b")).await;
            storage.flush_all().await;
        }

        std::fs::remove_file(dir.index_file()).unwrap();

        let reopened = StorageByHour::new(hour_key(), &dir.db_path());

        assert_eq!(stored_seconds(&reopened, 3).await, vec![10, 2000]);

        // And the rebuilt index still lets a bounded query skip the head of the hour.
        let late_only = reopened
            .get_by_service_name("my-service", "b", None, Some(at_second(1000)))
            .await;

        assert_eq!(late_only.len(), 1);
        assert_eq!(late_only[0].data, "b");
    }

    #[tokio::test]
    async fn get_by_service_name_filters_orders_and_limits() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        for second in 0..150i64 {
            push_one(&storage, metric(second, second, "GET /orders")).await;
        }
        push_one(&storage, metric(200, 999, "GET /users")).await;
        storage.flush_all().await;

        let found = storage
            .get_by_service_name("my-service", "GET /orders", None, None)
            .await;

        // Newest first, capped at the screen limit.
        assert_eq!(found.len(), LAST_EVENTS_LIMIT);
        assert_eq!(found[0].started, at_second(149));
        assert_eq!(found[LAST_EVENTS_LIMIT - 1].started, at_second(50));

        // The other action is not mixed in.
        assert!(found.iter().all(|dto| dto.data == "GET /orders"));
    }

    #[tokio::test]
    async fn get_by_service_name_filters_by_client_id() {
        let dir = TestDir::new();
        let storage = StorageByHour::new(hour_key(), &dir.db_path());

        let mut other = metric(10, 1, "GET /orders");
        other.client_id = Some("client-2".to_string());

        push_one(&storage, metric(11, 2, "GET /orders")).await;
        push_one(&storage, other).await;
        storage.flush_all().await;

        let found = storage
            .get_by_service_name("my-service", "GET /orders", Some("client-2"), None)
            .await;

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].client_id.as_deref(), Some("client-2"));
    }

    #[tokio::test]
    async fn a_torn_tail_is_dropped_and_the_rest_survives() {
        let dir = TestDir::new();

        {
            let storage = StorageByHour::new(hour_key(), &dir.db_path());
            push_one(&storage, metric(10, 5, "a")).await;
            push_one(&storage, metric(20, 5, "b")).await;
            storage.flush_all().await;
        }

        let data_file = dir.data_file();
        let index_file = dir.index_file();

        // A crash between the write and the fsync: half a record at the end, and an index
        // that never got updated.
        let mut bytes = std::fs::read(&data_file).unwrap();
        bytes.extend_from_slice(&[200u8, 0u8, 1u8, 2u8, 3u8]);
        std::fs::write(&data_file, &bytes).unwrap();
        std::fs::remove_file(&index_file).unwrap();

        let reopened = StorageByHour::new(hour_key(), &dir.db_path());

        assert_eq!(stored_seconds(&reopened, 5).await, vec![10, 20]);

        // The torn bytes are gone, so the next append does not land behind garbage.
        push_one(&reopened, metric(30, 5, "c")).await;
        reopened.flush_all().await;

        let again = StorageByHour::new(hour_key(), &dir.db_path());
        assert_eq!(stored_seconds(&again, 5).await, vec![10, 20, 30]);
    }
}
