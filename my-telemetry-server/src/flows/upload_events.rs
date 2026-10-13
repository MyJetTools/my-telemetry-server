use crate::{app_ctx::AppContext, db::*};

/// The metrics a writer sent go into the queue, and the background executor is signalled
/// to take them in: the writer is answered without waiting for it.
pub fn upload_events(app: &AppContext, events: Vec<MetricDto>) {
    if events.is_empty() {
        return;
    }

    app.to_write_queue.enqueue(events);
    app.queue_executor.trigger();
}
