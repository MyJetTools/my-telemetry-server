use std::sync::Arc;

use crate::{
    caches::{AppDataHourStatistics, AppDurationStatistics},
    db::*,
    reader_grpc::*,
    writer_grpc::*,
};

impl MetricDto {
    /// A metric as a writer sent it over gRPC, in the product scope the writer named
    pub fn from_grpc_event(event: TelemetryGrpcEvent, scope: Arc<str>) -> Self {
        let metric_tags = super::metric_tags::get(Some(event.tags));

        Self {
            scope,
            id: event.process_id,
            started: event.started_at,
            duration_micro: event.finished_at.saturating_sub(event.started_at),
            name: event.service_name,
            data: event.event_data,
            success: event.success,
            fail: event.fail,
            tags: metric_tags.tags,
            client_id: metric_tags.client_id,
        }
    }
}

/// The inverse of the mapping above - used to put a metric on disk, where
/// `TelemetryGrpcEvent` doubles as the storage format. The scope is not in it: it is the
/// folder the metric goes to.
///
/// `client_id` has no field of its own on the wire, so it goes back where it came from:
/// a `client_id` tag. `metric_tags::get` lifts it out again on the way in, so the
/// round-trip is lossless.
impl Into<TelemetryGrpcEvent> for MetricDto {
    fn into(mut self) -> TelemetryGrpcEvent {
        let mut tags = match self.tags.take() {
            Some(dto_tags) => {
                let mut result = Vec::with_capacity(dto_tags.len() + 1);
                for dto_tag in dto_tags {
                    result.push(EventGrpcTag {
                        key: dto_tag.key,
                        value: dto_tag.value,
                    });
                }
                result
            }
            None => Vec::with_capacity(1),
        };

        if let Some(client_id) = self.client_id.take() {
            tags.push(EventGrpcTag {
                key: super::CLIENT_ID_TAG.to_string(),
                value: client_id,
            });
        }

        TelemetryGrpcEvent {
            process_id: self.id,
            started_at: self.started,
            finished_at: self.started.saturating_add(self.duration_micro),
            service_name: self.name,
            event_data: self.data,
            success: self.success,
            fail: self.fail,
            tags,
        }
    }
}

impl From<AppDataHourStatistics> for AppActionGrpcModel {
    fn from(value: AppDataHourStatistics) -> Self {
        let total = value.success_amount + value.errors_amount;
        Self {
            data: value.data,
            min: value.min,
            avg: value.sum_of_duration / total,
            max: value.max,
            success: value.success_amount,
            error: value.errors_amount,
            total,
        }
    }
}

impl From<HourAppDataStatisticsDto> for AppActionGrpcModel {
    fn from(value: HourAppDataStatisticsDto) -> Self {
        let total = value.success_amount + value.errors_amount;
        Self {
            data: value.data,
            min: value.min,
            avg: value.sum_of_duration / total,
            max: value.max,
            success: value.success_amount,
            error: value.errors_amount,
            total,
        }
    }
}

impl From<AppDurationStatistics> for ServiceGrpcModel {
    fn from(value: AppDurationStatistics) -> Self {
        Self {
            id: value.name,
            avg: value.duration_micros / value.amount,
            amount: value.amount,
        }
    }
}

impl From<HourStatisticsDto> for ServiceGrpcModel {
    fn from(value: HourStatisticsDto) -> Self {
        Self {
            id: value.app,
            avg: value.duration_micros / value.amount,
            amount: value.amount,
        }
    }
}

impl PermanentMetricDto {
    /// A metric as the storage keeps it, as the table of the clients kept for good has
    /// it: the client in a column of its own, not among the tags
    pub fn of_record(record: &TelemetryGrpcEvent, client_id: &str) -> Self {
        let tags: Vec<EventTagDto> = record
            .tags
            .iter()
            .filter(|tag| tag.key != super::CLIENT_ID_TAG)
            .map(|tag| EventTagDto {
                key: tag.key.clone(),
                value: tag.value.clone(),
            })
            .collect();

        Self {
            id: record.process_id,
            client_id: client_id.to_string(),
            started: record.started_at,
            duration_micro: record.finished_at.saturating_sub(record.started_at),
            name: record.service_name.clone(),
            data: record.event_data.clone(),
            success: record.success.clone(),
            fail: record.fail.clone(),
            tags: (!tags.is_empty()).then_some(tags),
        }
    }
}

// What the storage keeps is the message of the writer: the `client_id` is among its tags
// already - the last of them - so a reader is given the tags as they are.

impl From<EventGrpcTag> for TagGrpcModel {
    fn from(value: EventGrpcTag) -> Self {
        Self {
            key: value.key,
            value: value.value,
        }
    }
}

impl From<TelemetryGrpcEvent> for AppDataGrpcModel {
    fn from(value: TelemetryGrpcEvent) -> Self {
        Self {
            process_id: value.process_id,
            started: value.started_at,
            duration: value.finished_at.saturating_sub(value.started_at),
            success: value.success,
            fail: value.fail,
            tags: value.tags.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<TelemetryGrpcEvent> for MetricEventGrpcModel {
    fn from(value: TelemetryGrpcEvent) -> Self {
        Self {
            started: value.started_at,
            duration: value.finished_at.saturating_sub(value.started_at),
            success: value.success,
            name: value.service_name,
            data: value.event_data,
            fail: value.fail,
            tags: value.tags.into_iter().map(Into::into).collect(),
        }
    }
}
