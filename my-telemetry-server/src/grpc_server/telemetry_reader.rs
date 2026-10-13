use std::sync::Arc;

use crate::metrics_storage::{hour_start_of_key, MICROS_PER_SECOND};
use crate::product_scope::{scope_of_reader, DEFAULT_PRODUCT_SCOPE};
use crate::reader_grpc::telemetry_reader_server::TelemetryReader;
use crate::reader_grpc::*;

use super::server::GrpcService;
use my_grpc_extensions::server::generate_server_stream;

/// The product scope a request reads: the default one when it names none
fn requested_scope(scope: Option<&str>) -> Result<Arc<str>, tonic::Status> {
    scope_of_reader(scope).map_err(tonic::Status::invalid_argument)
}

#[tonic::async_trait]
impl TelemetryReader for GrpcService {
    async fn get_product_scopes(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<GetProductScopesResponse>, tonic::Status> {
        Ok(tonic::Response::new(GetProductScopesResponse {
            default_scope: DEFAULT_PRODUCT_SCOPE.to_string(),
            scopes: self.app.repo.scopes().await,
        }))
    }

    generate_server_stream!(stream_name:"GetAvailableHoursAgoStream", item_name:"AvailableFileGrpcModel");

    async fn get_available_hours_ago(
        &self,
        request: tonic::Request<GetAvailableHoursRequest>,
    ) -> Result<tonic::Response<Self::GetAvailableHoursAgoStream>, tonic::Status> {
        let request = request.into_inner();
        let scope = requested_scope(request.product_scope.as_deref())?;

        let response = crate::flows::get_available_hours_ago(&self.app, &scope).await;

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            response.into_iter(),
            |dto| dto,
        )
        .await
    }

    generate_server_stream!(stream_name:"GetAppsStream", item_name:"ServiceGrpcModel");

    async fn get_apps(
        &self,
        request: tonic::Request<GetAppsRequest>,
    ) -> Result<tonic::Response<Self::GetAppsStream>, tonic::Status> {
        let request = request.into_inner();
        let scope = requested_scope(request.product_scope.as_deref())?;

        let overview: Vec<ServiceGrpcModel> =
            crate::flows::get_hour_app_statistics(&self.app, request.hour_key.into(), &scope).await;

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            overview.into_iter(),
            |dto| dto,
        )
        .await
    }

    generate_server_stream!(stream_name:"GetAppActionsStream", item_name:"AppActionGrpcModel");

    async fn get_app_actions(
        &self,
        request: tonic::Request<GetByAppRequest>,
    ) -> Result<tonic::Response<Self::GetAppActionsStream>, tonic::Status> {
        let request = request.into_inner();
        let scope = requested_scope(request.product_scope.as_deref())?;

        let result: Vec<AppActionGrpcModel> = crate::flows::get_hour_app_data_statistics(
            &self.app,
            request.hour_key.into(),
            &scope,
            &request.app_id,
        )
        .await;

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            result.into_iter(),
            |dto| dto,
        )
        .await
    }

    generate_server_stream!(stream_name:"GetAppEventsByActionStream", item_name:"AppDataGrpcModel");

    async fn get_app_events_by_action(
        &self,
        request: tonic::Request<GetAppEventsByActionRequest>,
    ) -> Result<tonic::Response<Self::GetAppEventsByActionStream>, tonic::Status> {
        let request = request.into_inner();
        let scope = requested_scope(request.product_scope.as_deref())?;

        // A number which is not a key of an hour: there are no metrics of it
        let events = match hour_start_of_key(request.hour_key) {
            Some(hour_start) => {
                // A second of the hour: 0 is "from its start", and so is what is not one
                let from_started = (1..3600)
                    .contains(&request.from_sec_within_hour)
                    .then(|| hour_start + request.from_sec_within_hour * MICROS_PER_SECOND);

                let client_id = if request.client_id.is_empty() {
                    None
                } else {
                    Some(request.client_id)
                };

                self.app
                    .repo
                    .search_action(
                        scope,
                        hour_start,
                        request.app_id,
                        request.data,
                        client_id,
                        from_started,
                    )
                    .await
            }
            None => Vec::new(),
        };

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            events.into_iter(),
            |event| event.into(),
        )
        .await
    }

    generate_server_stream!(stream_name:"GetByProcessIdStream", item_name:"MetricEventGrpcModel");

    async fn get_by_process_id(
        &self,
        request: tonic::Request<GetByProcessIdRequest>,
    ) -> Result<tonic::Response<Self::GetByProcessIdStream>, tonic::Status> {
        let request = request.into_inner();

        let events = match hour_start_of_key(request.hour_key) {
            Some(hour_start) => {
                self.app
                    .repo
                    .search_process(hour_start, request.process_id)
                    .await
            }
            None => Vec::new(),
        };

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            events.into_iter(),
            |event| event.into(),
        )
        .await
    }

    async fn get_tech_metrics(
        &self,
        _: tonic::Request<()>,
    ) -> Result<tonic::Response<TechMetricsGrpcModel>, tonic::Status> {
        let response = {
            let (queue_size, queue_capacity) = self.app.to_write_queue.get_size_and_capacity();

            let storage_window_size = self.app.repo.window_len();

            let cache_read_access = self.app.cache.lock().await;

            let app_data_size = cache_read_access
                .statistics_by_app_and_data
                .get_size_and_capacity();

            let app_data_hour_size = cache_read_access
                .statistics_by_app_and_data
                .get_queue_hours_size();

            let (user_id_links_size, user_id_links_capacity) = cache_read_access
                .process_id_user_id_links
                .get_size_and_capacity();

            TechMetricsGrpcModel {
                app_data_hours_size: app_data_hour_size.0 as u64,
                app_data_to_persist_hours_size: app_data_hour_size.1 as u64,
                queue_size: queue_size as u64,
                queue_capacity: queue_capacity as u64,
                // There is no queue by process any more: the metrics are taken in at once
                queue_by_process_size: 0,
                queue_by_process_capacity: 0,
                user_id_links_size: user_id_links_size as u64,
                app_data_size: app_data_size.0 as u64,
                app_data_capacity: app_data_size.1 as u64,
                user_id_links_capacity: user_id_links_capacity as u64,
                storage_window_size: storage_window_size as u64,
            }
        };

        Ok(tonic::Response::new(response))
    }

    async fn add_permanent_user(
        &self,
        request: tonic::Request<AddPermanentUserGrpcRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let request = request.into_inner();
        crate::flows::add_permanent_user(&self.app, request.user_id).await;
        Ok(tonic::Response::new(()))
    }

    async fn delete_permanent_user(
        &self,
        request: tonic::Request<DeletePermanentUserGrpcRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let request = request.into_inner();
        crate::flows::delete_permanent_user(&self.app, request.user_id).await;
        Ok(tonic::Response::new(()))
    }

    generate_server_stream!(stream_name:"GetPermanentUsersStream", item_name:"PermanentUserGrpcModel");

    async fn get_permanent_users(
        &self,
        _: tonic::Request<()>,
    ) -> Result<tonic::Response<Self::GetPermanentUsersStream>, tonic::Status> {
        let dto_data = crate::flows::get_permanent_users(&self.app).await;

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            dto_data.into_iter(),
            |model| PermanentUserGrpcModel {
                user_id: model.user,
                added: model.created,
                status: model.status,
            },
        )
        .await
    }

    async fn ping(&self, _: tonic::Request<()>) -> Result<tonic::Response<()>, tonic::Status> {
        Ok(tonic::Response::new(()))
    }
}
