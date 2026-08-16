use std::sync::Arc;

use rust_extensions::{date_time::DateTimeAsMicroseconds, MyTimerTick, RepeatTimerIteration};

use crate::{
    app_ctx::{AppContext, StatisticsCache},
    db::{MetricDto, PermanentMetricDto},
    storage_by_hour::group_by_hour,
    to_write_queue::MetricsChunkByProcessId,
};

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
        let started = DateTimeAsMicroseconds::now();
        let seconds_to_flush = self.app.settings_reader.get_seconds_to_flush().await;
        let window_seconds = self.app.settings_reader.get_window_seconds().await;

        let mut do_gc = true;

        while let Some(chunks) = self
            .app
            .to_write_queue
            .get_events_to_write(1000, seconds_to_flush)
            .await
        {
            // `get_events_to_write` answers `Some(vec![])` when nothing has aged out yet,
            // so without this the loop spins on the queue lock until the 20 second guard
            // below fires - every tick.
            if chunks.is_empty() {
                break;
            }

            let mut events_to_write = Vec::with_capacity(1000);

            {
                let mut cache_access = self.app.cache.lock().await;
                for chunk in chunks {
                    populate_client_id(chunk, &mut cache_access, &mut events_to_write).await;
                }
            }

            // An hour is its own pair of files, so the batch is split before anything is
            // handed over. Statistics are updated off the same grouping, by reference -
            // the metrics themselves move on to the storage untouched.
            let by_hour = group_by_hour(events_to_write);

            let mut permanent_items: Vec<PermanentMetricDto> = Vec::new();
            let mut to_push = Vec::with_capacity(by_hour.len());

            {
                let mut cache_write_access = self.app.cache.lock().await;

                for (hour_key, grouped) in by_hour {
                    cache_write_access
                        .statistics_by_app_and_data
                        .update(hour_key, &grouped);

                    for metric_dto in grouped.iter() {
                        cache_write_access
                            .event_amount_by_hours
                            .inc(hour_key, metric_dto);

                        if let Some(client_id) = &metric_dto.client_id {
                            if cache_write_access
                                .permanent_users_list
                                .is_permanent(client_id)
                            {
                                permanent_items.push(metric_dto.clone().into());
                            }
                        }
                    }

                    to_push.push((hour_key, grouped));
                }

                if do_gc {
                    cache_write_access.process_id_user_id_links.gc();
                    do_gc = false;
                }
            }

            for (hour_key, items) in to_push {
                self.app.repo.push(hour_key, items).await;
            }

            if permanent_items.len() > 0 {
                self.app.permanent_metrics.insert(&permanent_items).await
            }

            if (DateTimeAsMicroseconds::now() - started).get_full_seconds() >= 20 {
                break;
            }
        }

        // Everything that has aged past the window goes to disk. This is the only place
        // metrics reach the filesystem, and it is what keeps the data files append-only.
        let mut flush_before = DateTimeAsMicroseconds::now();
        flush_before.add_seconds(-window_seconds);

        self.app.repo.flush(flush_before.unix_microseconds).await;

        RepeatTimerIteration::WithInterval
    }
}

async fn populate_client_id<'s>(
    chunk: MetricsChunkByProcessId,
    cache: &'s mut StatisticsCache,
    out_put: &mut Vec<MetricDto>,
) {
    if let Some(client_id) = cache
        .process_id_user_id_links
        .resolve_user_id(chunk.process_id)
    {
        for mut metric in chunk.items {
            if metric.client_id.is_none() {
                metric.client_id = Some(client_id.to_string());
            }

            out_put.push(metric);
        }
        return;
    }

    for metric in chunk.items {
        out_put.push(metric);
    }
}
