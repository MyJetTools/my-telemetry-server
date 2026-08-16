use std::{collections::BTreeMap, sync::Arc};

use my_sqlite::{SqlLiteConnection, SqlLiteConnectionBuilder};
use rust_extensions::date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey};
use tokio::sync::Mutex;

use super::dto::MetricDto;

pub struct SqlLitePoolItem {
    pub last_access: DateTimeAsMicroseconds,
    pub connection: Arc<Mutex<SqlLiteConnection>>,
    pub file_name: String,
}

impl SqlLitePoolItem {
    pub async fn new(file_name: String) -> Self {
        let connection = SqlLiteConnectionBuilder::new(file_name.to_string())
            .create_table_if_no_exists::<MetricDto>(super::TABLE_NAME)
            .build()
            .await
            .unwrap();

        Self {
            last_access: DateTimeAsMicroseconds::now(),
            connection: Arc::new(Mutex::new(connection)),
            file_name,
        }
    }
}

#[derive(Default)]
pub struct SqlLitePoolSingleThreaded {
    pool: BTreeMap<IntervalKey<HourKey>, SqlLitePoolItem>,
    being_deleted: Option<IntervalKey<HourKey>>,
}

pub struct SqlLitePool {
    file_name_prefix: String,
    pool: Mutex<SqlLitePoolSingleThreaded>,
}

impl SqlLitePool {
    pub fn new(file_name_prefix: String) -> Self {
        Self {
            file_name_prefix,
            pool: Mutex::new(SqlLitePoolSingleThreaded::default()),
        }
    }

    pub async fn get_for_read_access(
        &self,
        hour_key: IntervalKey<HourKey>,
    ) -> Option<Arc<Mutex<SqlLiteConnection>>> {
        let mut write_access = self.pool.lock().await;

        if let Some(being_deleted) = write_access.being_deleted {
            if being_deleted == hour_key {
                return None;
            }
        }

        if let Some(pool_item) = write_access.pool.get_mut(&hour_key) {
            pool_item.last_access = DateTimeAsMicroseconds::now();

            return Some(pool_item.connection.clone());
        }

        let file_name = compile_file_name(&self.file_name_prefix, hour_key);

        let file_info = tokio::fs::metadata(&file_name).await;

        if file_info.is_err() {
            return None;
        }

        let item = SqlLitePoolItem::new(file_name).await;

        let result = item.connection.clone();
        write_access.pool.insert(hour_key, item);

        Some(result)
    }

    pub async fn get_for_write_access(
        &self,
        hour_key: IntervalKey<HourKey>,
    ) -> Arc<Mutex<SqlLiteConnection>> {
        let mut write_access = self.pool.lock().await;

        let file_name = compile_file_name(&self.file_name_prefix, hour_key);

        let item = SqlLitePoolItem::new(file_name).await;

        let result = item.connection.clone();
        write_access.pool.insert(hour_key, item);

        result
    }
    /*
    pub async fn get_last(&self) -> Option<Arc<Mutex<SqlLiteConnection>>> {
        let mut write_access = self.pool.lock().await;
        if let Some((_, itm)) = write_access.iter_mut().last() {
            itm.last_access = DateTimeAsMicroseconds::now();
            return Some(itm.connection.clone());
        }
        None
    }


       pub async fn get_all(&self) -> Vec<Arc<Mutex<SqlLiteConnection>>> {
           let mut result = Vec::new();
           let write_access = self.pool.lock().await;
           for (_, item) in write_access.iter() {
               result.insert(0, item.connection.clone());
           }
           result
       }
    */
    pub async fn gc_file(&self, gc_file: IntervalKey<HourKey>) {
        let mut write_access = self.pool.lock().await;

        write_access.being_deleted = Some(gc_file);

        if let Some(item) = write_access.pool.remove(&gc_file) {
            println!(
                "File {} for hour Key {} is removed from the pool",
                item.file_name,
                gc_file.to_i64()
            );
        }
    }
}

fn compile_file_name(file_name_prefix: &str, hour_key: IntervalKey<HourKey>) -> String {
    format!("{}-{}.db", file_name_prefix, hour_key.to_i64())
}
