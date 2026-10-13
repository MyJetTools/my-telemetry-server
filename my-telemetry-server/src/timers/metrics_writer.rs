use std::sync::Arc;

use rust_extensions::{MyTimerTick, RepeatTimerIteration};

use crate::app_ctx::AppContext;

/// Writes the seconds which have waited in the memory of the storage. This is the only
/// place metrics reach the filesystem - but for the stop of the process. What comes is
/// taken in without a timer: see [`crate::background::MetricsQueueJob`].
pub struct MetricsWriter {
    app: Arc<AppContext>,
}

impl MetricsWriter {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
#[async_trait::async_trait]
impl MyTimerTick for MetricsWriter {
    async fn tick(&self) -> RepeatTimerIteration {
        crate::flows::write_ready_metrics(&self.app, false).await;

        RepeatTimerIteration::WithInterval
    }
}
