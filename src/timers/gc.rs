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
        let duration = self.app.settings_reader.get_hours_to_gc().await;

        let hour_key: IntervalKey<HourKey> = DateTimeAsMicroseconds::now().sub(duration).into();

        //println!("GC hour is: {}", hour_key.to_i64());

        crate::scripts::gc_files(&self.app, hour_key).await;

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
