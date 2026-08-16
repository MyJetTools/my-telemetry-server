use std::collections::BTreeMap;
use std::sync::Arc;

use rust_extensions::date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey};
use tokio::sync::Mutex;

use crate::db::MetricDto;

use super::StorageByHour;

/// Every hour currently open, keyed by hour. Replaces the turso `HourDbPool`.
///
/// The map is only ever held long enough to hand out an `Arc` - all the real work happens
/// inside [`StorageByHour`], behind its own lock, so one busy hour never blocks another.
pub struct MetricsStorage {
    db_path: String,
    hours: Mutex<BTreeMap<IntervalKey<HourKey>, Arc<StorageByHour>>>,
}

impl MetricsStorage {
    pub fn new(db_path: String) -> Self {
        Self {
            db_path,
            hours: Mutex::new(BTreeMap::new()),
        }
    }

    /// Hands metrics to the hour that owns them. They land in that hour's in-memory
    /// window - nothing reaches disk until [`Self::flush`].
    pub async fn push(&self, hour_key: IntervalKey<HourKey>, items: Vec<MetricDto>) {
        let storage = self.get_or_create(hour_key).await;
        storage.push_range(items).await;
    }

    /// Moves everything older than `flush_before` onto disk, across every open hour.
    ///
    /// An hour that has rolled over needs no special handling: all of its records are
    /// older than the boundary, so they all drain on the next tick.
    pub async fn flush(&self, flush_before: i64) {
        let hours: Vec<Arc<StorageByHour>> = self.hours.lock().await.values().cloned().collect();

        for storage in hours {
            storage.flush(flush_before).await;
        }
    }

    /// Empties every window onto disk regardless of age.
    ///
    /// Called on shutdown: without it an ordinary restart would drop whatever the window
    /// was holding, so every deploy would cost a window's worth of telemetry.
    pub async fn flush_all(&self) {
        let hours: Vec<Arc<StorageByHour>> = self.hours.lock().await.values().cloned().collect();

        for storage in hours {
            storage.flush_all().await;
        }
    }

    pub async fn get_by_process_id(
        &self,
        hour_key: IntervalKey<HourKey>,
        process_id: i64,
    ) -> Vec<MetricDto> {
        let Some(storage) = self.get(hour_key).await else {
            return vec![];
        };

        storage.get_by_process_id(process_id).await
    }

    pub async fn get_by_service_name(
        &self,
        hour_key: IntervalKey<HourKey>,
        service_name: &str,
        data: &str,
        client_id: Option<&str>,
        started: Option<i64>,
    ) -> Vec<MetricDto> {
        let Some(storage) = self.get(hour_key).await else {
            return vec![];
        };

        storage
            .get_by_service_name(service_name, data, client_id, started)
            .await
    }

    /// Events accepted but not yet on disk, across every open hour. This is what a crash
    /// would cost.
    pub async fn window_len(&self) -> usize {
        let hours: Vec<Arc<StorageByHour>> = self.hours.lock().await.values().cloned().collect();

        let mut result = 0;

        for storage in hours {
            result += storage.window_len().await;
        }

        result
    }

    /// Drops the hour and returns the folder that backs it, so the caller can delete it.
    ///
    /// The map stays locked across the close: a reader that arrives afterwards finds no
    /// entry and no folder, which is an empty result rather than a half-deleted hour.
    pub async fn gc(&self, hour_key: IntervalKey<HourKey>) -> Option<String> {
        let mut write_access = self.hours.lock().await;

        let storage = write_access.remove(&hour_key)?;

        Some(storage.close().await)
    }

    async fn get_or_create(&self, hour_key: IntervalKey<HourKey>) -> Arc<StorageByHour> {
        let mut write_access = self.hours.lock().await;

        if let Some(storage) = write_access.get(&hour_key) {
            return storage.clone();
        }

        let storage = Arc::new(StorageByHour::new(hour_key, &self.db_path));

        write_access.insert(hour_key, storage.clone());

        storage
    }

    /// Read side. Unlike [`Self::get_or_create`] this refuses to register an hour that has
    /// no files - `hour_key` comes from the client, and an unbounded map keyed by whatever
    /// it asks for is a way to grow the process without writing a single metric.
    async fn get(&self, hour_key: IntervalKey<HourKey>) -> Option<Arc<StorageByHour>> {
        let mut write_access = self.hours.lock().await;

        if let Some(storage) = write_access.get(&hour_key) {
            return Some(storage.clone());
        }

        let data_file_name = super::data_file_name(self.db_path.as_str(), hour_key);

        if !std::path::Path::new(data_file_name.as_str()).exists() {
            return None;
        }

        let storage = Arc::new(StorageByHour::new(hour_key, &self.db_path));

        write_access.insert(hour_key, storage.clone());

        Some(storage)
    }
}

/// Splits a batch into the hours that own it. Metrics arrive in one stream and an hour is
/// a separate pair of files, so this is where the boundary gets drawn.
pub fn group_by_hour(items: Vec<MetricDto>) -> BTreeMap<IntervalKey<HourKey>, Vec<MetricDto>> {
    let mut result: BTreeMap<IntervalKey<HourKey>, Vec<MetricDto>> = BTreeMap::new();

    for dto in items {
        let hour_key: IntervalKey<HourKey> = DateTimeAsMicroseconds::new(dto.started).into();

        result.entry(hour_key).or_default().push(dto);
    }

    result
}
