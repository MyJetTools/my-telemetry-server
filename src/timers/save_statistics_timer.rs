use std::sync::Arc;

use rust_extensions::{MyTimerTick, RepeatTimerIteration};

use crate::app_ctx::AppContext;

pub struct SaveStatisticsTimer {
    app: Arc<AppContext>,
}

impl SaveStatisticsTimer {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl MyTimerTick for SaveStatisticsTimer {
    async fn tick(&self) -> RepeatTimerIteration {
        let (app_data_metrics, app_hour_metrics) = {
            let mut metrics_access = self.app.cache.lock().await;

            let app_data_metrics = metrics_access.statistics_by_app_and_data.get_to_persist();

            let app_hour_metrics = metrics_access.event_amount_by_hours.get_to_persist();

            (app_data_metrics, app_hour_metrics)
        };

        if let Some(app_data_metrics) = app_data_metrics {
            crate::scripts::write_hour_app_data_statistics(&self.app, app_data_metrics).await;
        }

        if let Some(app_hour_metrics) = app_hour_metrics {
            crate::scripts::write_hour_statistics_to_db(&self.app, app_hour_metrics).await;
        }

        RepeatTimerIteration::WithInterval
    }
}
