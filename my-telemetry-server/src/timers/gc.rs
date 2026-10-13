use std::{sync::Arc, time::Duration};

use rust_extensions::{
    date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey},
    MyTimerTick, RepeatTimerIteration,
};

use crate::app_ctx::AppContext;

pub struct GcTimer {
    app: Arc<AppContext>,
}

impl GcTimer {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl MyTimerTick for GcTimer {
    async fn tick(&self) -> RepeatTimerIteration {
        let hours_to_gc = self.app.settings_reader.get_hours_to_gc().await;
        let storage = self.app.repo.clone();

        // Folders are deleted and key indexes written: off the workers of the runtime
        let messages = tokio::task::spawn_blocking(move || {
            storage.gc(DateTimeAsMicroseconds::now(), hours_to_gc)
        })
        .await
        .unwrap();

        for message in messages {
            println!("{}", message);
        }

        let cache_gc_hour_key: IntervalKey<HourKey> = DateTimeAsMicroseconds::now()
            .sub(Duration::from_secs(60 * 60 * 2))
            .into();

        let mut cache_access = self.app.cache.lock().await;

        cache_access
            .statistics_by_app_and_data
            .gc_old_data(cache_gc_hour_key);

        cache_access
            .event_amount_by_hours
            .gc_old_data(cache_gc_hour_key);

        RepeatTimerIteration::WithInterval
    }
}
