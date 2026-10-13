// What my-telemetry-server answers over gRPC -> what the wasm client is sent. The shapes
// match one to one; only the names differ.

use my_telemetry_ui_shared::*;

use crate::reader_grpc::*;

impl From<TagGrpcModel> for TagHttpModel {
    fn from(value: TagGrpcModel) -> Self {
        Self {
            key: value.key,
            value: value.value,
        }
    }
}

impl From<GetProductScopesResponse> for ProductScopesHttpModel {
    fn from(value: GetProductScopesResponse) -> Self {
        Self {
            default_scope: value.default_scope,
            scopes: value.scopes,
        }
    }
}

impl From<ServiceGrpcModel> for ServiceHttpModel {
    fn from(value: ServiceGrpcModel) -> Self {
        Self {
            id: value.id,
            avg: value.avg,
            amount: value.amount,
        }
    }
}

impl From<AppActionGrpcModel> for ServiceOverviewContract {
    fn from(value: AppActionGrpcModel) -> Self {
        Self {
            data: value.data,
            min: value.min,
            max: value.max,
            avg: value.avg,
            success: value.success,
            error: value.error,
            total: value.total,
        }
    }
}

impl From<AppDataGrpcModel> for MetricHttpModel {
    fn from(value: AppDataGrpcModel) -> Self {
        Self {
            id: value.process_id,
            started: value.started,
            duration: value.duration,
            success: value.success,
            error: value.fail,
            tags: value.tags.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<MetricEventGrpcModel> for MetricByProcessModel {
    fn from(value: MetricEventGrpcModel) -> Self {
        Self {
            id: value.name,
            data: value.data,
            started: value.started,
            duration: value.duration,
            success: value.success,
            error: value.fail,
            tags: value.tags.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<TechMetricsGrpcModel> for TechMetricsHttpModel {
    fn from(value: TechMetricsGrpcModel) -> Self {
        Self {
            app_data_hours_size: value.app_data_hours_size,
            app_data_to_persist_hours_size: value.app_data_to_persist_hours_size,
            queue_size: value.queue_size,
            queue_capacity: value.queue_capacity,
            queue_by_process_size: value.queue_by_process_size,
            queue_by_process_capacity: value.queue_by_process_capacity,
            user_id_links_size: value.user_id_links_size,
            user_id_links_capacity: value.user_id_links_capacity,
            app_data_size: value.app_data_size,
            app_data_capacity: value.app_data_capacity,
            storage_window_size: value.storage_window_size,
        }
    }
}
