use std::sync::Arc;

use rust_extensions::background_executor::{BackgroundJob, RepeatIteration};

use crate::app_ctx::AppContext;

/// Takes in the metrics the writers put into the queue - as soon as one of them signals
/// the executor, not by a timer.
pub struct MetricsQueueJob {
    app: Arc<AppContext>,
}

impl MetricsQueueJob {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl BackgroundJob for MetricsQueueJob {
    async fn execute(&self) -> RepeatIteration {
        if crate::flows::take_in_queued_metrics(&self.app).await {
            RepeatIteration::Yes
        } else {
            RepeatIteration::No
        }
    }
}
