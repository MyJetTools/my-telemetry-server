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
        crate::flows::save_statistics(&self.app).await;

        RepeatTimerIteration::WithInterval
    }
}
