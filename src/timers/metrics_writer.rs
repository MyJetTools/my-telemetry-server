use std::sync::Arc;

use rust_extensions::{MyTimerTick, StopWatch};

use crate::{
    app_ctx::{AppContext, StatisticsCache},
    db::MetricDto,
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
    async fn tick(&self) {
        while let Some(chunks) = self.app.to_write_queue.get_events_to_write(1000).await {
            let mut events_to_write = Vec::new();

            {
                let mut lazy_lock = crate::lazy_lock::LazyLock::new(&self.app.cache);
                for chunk in chunks {
                    populate_client_id(chunk, &mut lazy_lock, &mut events_to_write).await;
                }
            }

            //let events_amount = events_to_write.len();
            let mut sw = StopWatch::new();
            sw.start();
            let items = self.app.repo.insert(events_to_write).await;
            sw.pause();

            /*
            println!(
                "MetricsWriter written {} metrics in: {:?}",
                events_amount,
                sw.duration()
            );
             */

            let mut cache_write_access = self.app.cache.lock().await;

            for (interval_key, grouped) in &items {
                cache_write_access
                    .statistics_by_hour_and_service_name
                    .update(*interval_key, grouped);

                for metric_dto in grouped {
                    cache_write_access
                        .event_amount_by_hours
                        .inc(*interval_key, metric_dto);
                }
            }
        }
    }
}

async fn populate_client_id<'s>(
    mut chunk: MetricsChunkByProcessId,
    cache: &'s mut crate::lazy_lock::LazyLock<'_, StatisticsCache>,
    out_put: &mut Vec<MetricDto>,
) {
    let client_id = chunk.client_id.take();

    let mut cash_has_value = false;

    if client_id.is_none() {
        if let Some(resolved_client_id) = cache
            .get()
            .await
            .process_id_user_id_links
            .resolve_user_id(chunk.process_id)
        {
            chunk.client_id = Some(resolved_client_id.to_string());
            cash_has_value = true;
        }
    }

    for mut metric in chunk.items {
        if let Some(metric_client_id) = metric.client_id.as_ref() {
            if !cash_has_value {
                cache
                    .get_mut()
                    .await
                    .process_id_user_id_links
                    .update(chunk.process_id, metric_client_id);
                cash_has_value = true;
            }
        } else {
            metric.client_id = client_id.clone();
        }

        out_put.push(metric);
    }
}
