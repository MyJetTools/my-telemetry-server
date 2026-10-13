use std::{collections::BTreeMap, sync::Arc};

use rust_extensions::date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey};

use crate::{
    app_ctx::AppContext,
    db::MetricDto,
    metrics_storage::{hour_key_of, hour_start_of, local_time_is_over, time_to_keep},
};

/// One pass takes in this many metrics at most, then the next one goes on: the lock of
/// the statistics is let go between them
const MAX_BATCH: usize = 10_000;

/// Takes what is in the queue: remembers which client each process is of, counts the
/// metrics into the statistics and hands them to the storage, where they wait in memory
/// to be written. Returns `true` when the queue had more than one pass takes.
///
/// Nothing waits for anything here. What has to wait - the `client_id` of a process, which
/// comes with one of its events, often the last one - waits in the storage: see
/// [`super::write_ready_metrics`].
pub async fn take_in_queued_metrics(app: &AppContext) -> bool {
    // One pass at a time - the one of the executor, or the one a stop makes: a stop does
    // not write the storage out while a pass still holds metrics it took from the queue.
    // Held across the lock of the statistics: `tokio::sync::Mutex`, not `parking_lot`.
    let _pass = app.queue_pass.lock().await;

    let mut metrics = app.to_write_queue.take(MAX_BATCH);

    if metrics.is_empty() {
        return false;
    }

    let has_more = metrics.len() == MAX_BATCH;

    let too_late = keep_what_is_stored(
        &mut metrics,
        DateTimeAsMicroseconds::now().unix_microseconds,
    );

    if let Some((metrics, hour_key)) = too_late {
        println!(
            "{} metrics are dropped: the hour {} of one of them - and the hours of the others - are not written into any more",
            metrics, hour_key
        );
    }

    // The statistics are kept by hour and product scope, so the batch is split before
    // anything is handed over. They are updated by reference - the metrics themselves
    // move on to the storage untouched.
    let by_hour_and_scope = group_by_hour_and_scope(metrics);

    {
        let mut cache = app.cache.lock().await;

        for ((hour_key, scope), metrics) in &by_hour_and_scope {
            cache
                .statistics_by_app_and_data
                .update(*hour_key, scope, metrics);

            for metric in metrics {
                cache.event_amount_by_hours.inc(*hour_key, scope, metric);

                if let Some(client_id) = &metric.client_id {
                    cache.process_id_user_id_links.update(metric.id, client_id);
                }
            }
        }
    }

    // Into the memory of the storage: it writes them once they have waited there
    for ((_, scope), metrics) in by_hour_and_scope {
        app.repo.push(&scope, metrics.into_iter().map(Into::into));
    }

    has_more
}

/// Leaves the metrics the storage keeps, at the time it keeps each at: the statistics
/// count what is stored, under the hour it is stored in. A time which is whatever its
/// writer gave - none, or one a day ahead - is the time the metric came; a metric of an
/// hour which is not written into any more is dropped here. Returns how many were, and
/// the hour of one of them.
fn keep_what_is_stored(metrics: &mut Vec<MetricDto>, now: i64) -> Option<(usize, i64)> {
    let mut too_late = None;

    metrics.retain_mut(|metric| {
        metric.started = time_to_keep(metric.started, now);

        let hour_start = hour_start_of(metric.started);

        if !local_time_is_over(hour_start, now) {
            return true;
        }

        let (dropped, _) = too_late.get_or_insert((0, hour_key_of(hour_start)));
        *dropped += 1;

        false
    });

    too_late
}

/// Splits a batch by the hour and the product scope which own each metric: the statistics
/// are kept by the two.
fn group_by_hour_and_scope(
    items: Vec<MetricDto>,
) -> BTreeMap<(IntervalKey<HourKey>, Arc<str>), Vec<MetricDto>> {
    let mut result: BTreeMap<(IntervalKey<HourKey>, Arc<str>), Vec<MetricDto>> = BTreeMap::new();

    for dto in items {
        let hour_key: IntervalKey<HourKey> = DateTimeAsMicroseconds::new(dto.started).into();

        result
            .entry((hour_key, dto.scope.clone()))
            .or_default()
            .push(dto);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics_storage::HOUR_MICROS;

    /// 2023-11-14 22:13:20 UTC
    const NOW: i64 = 1_700_000_000_000_000;

    fn metric(started: i64) -> MetricDto {
        MetricDto {
            scope: "default".into(),
            id: 1,
            started,
            duration_micro: 700,
            name: "api".to_string(),
            data: "call".to_string(),
            success: None,
            fail: None,
            tags: None,
            client_id: None,
        }
    }

    #[test]
    fn a_time_which_is_whatever_its_writer_gave_does_not_cost_the_batch() {
        let mut metrics = vec![
            metric(NOW - 5),
            metric(0),
            metric(i64::MIN),
            metric(i64::MAX),
            metric(-1),
            // Two hours and more after its hour ended: the storage takes no more of it
            metric(NOW - 4 * HOUR_MICROS),
            metric(NOW - 30 * HOUR_MICROS),
        ];

        let too_late = keep_what_is_stored(&mut metrics, NOW);

        let started: Vec<i64> = metrics.iter().map(|metric| metric.started).collect();
        assert_eq!(started, [NOW - 5, NOW, NOW, NOW, NOW]);
        assert_eq!(too_late, Some((2, 2023111418)));

        // And every one which is left has an hour to be counted under
        let grouped = group_by_hour_and_scope(metrics);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped.values().next().unwrap().len(), 5);

        let mut on_time = vec![metric(NOW - 5)];
        assert_eq!(keep_what_is_stored(&mut on_time, NOW), None);
    }
}
