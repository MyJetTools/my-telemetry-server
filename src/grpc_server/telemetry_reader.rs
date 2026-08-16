use crate::reader_grpc::telemetry_reader_server::TelemetryReader;
use crate::reader_grpc::*;

use super::server::GrpcService;
use my_grpc_extensions::server::generate_server_stream;
use rust_extensions::date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey};

#[tonic::async_trait]
impl TelemetryReader for GrpcService {
    generate_server_stream!(stream_name:"GetAvailableHoursAgoStream", item_name:"AvailableFileGrpcModel");

    async fn get_available_hours_ago(
        &self,
        _request: tonic::Request<()>,
    ) -> Result<tonic::Response<Self::GetAvailableHoursAgoStream>, tonic::Status> {
        // let request = request.into_inner();
        let response = crate::flows::get_available_hours_ago(&self.app).await;

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
        let overview: Vec<ServiceGrpcModel> =
            crate::flows::get_hour_app_statistics(&self.app, request.hour_key.into()).await;

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

        let result: Vec<AppActionGrpcModel> = crate::flows::get_hour_app_data_statistics(
            &self.app,
            request.hour_key.into(),
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

        let hour_key: IntervalKey<HourKey> = request.hour_key.into();
        let from_started = if request.from_sec_within_hour == 0 {
            None
        } else {
            let mut dt: DateTimeAsMicroseconds = hour_key.try_to_date_time().unwrap();
            dt.add_seconds(request.from_sec_within_hour);
            Some(dt.unix_microseconds)
        };

        let client_id = if request.client_id.is_empty() {
            None
        } else {
            Some(request.client_id.as_str())
        };

        let dto_data = self
            .app
            .repo
            .get_by_service_name(
                hour_key,
                &request.app_id,
                &request.data,
                client_id,
                from_started,
            )
            .await;

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            dto_data.into_iter(),
            |dto| dto.into(),
        )
        .await
    }

    generate_server_stream!(stream_name:"GetByProcessIdStream", item_name:"MetricEventGrpcModel");

    async fn get_by_process_id(
        &self,
        request: tonic::Request<GetByProcessIdRequest>,
    ) -> Result<tonic::Response<Self::GetByProcessIdStream>, tonic::Status> {
        let request = request.into_inner();

        let dto_data = self
            .app
            .repo
            .get_by_process_id(request.hour_key.into(), request.process_id)
            .await;

        my_grpc_extensions::grpc_server_streams::send_from_iterator_with_transformation(
            dto_data.into_iter(),
            |dto| dto.into(),
        )
        .await
    }

    async fn get_tech_metrics(
        &self,
        _: tonic::Request<()>,
    ) -> Result<tonic::Response<TechMetricsGrpcModel>, tonic::Status> {
        let response = {
            let queue_and_capacity = self
                .app
                .to_write_queue
                .get_queue_and_capacity_and_by_process_capacity()
                .await;

            let storage_window_size = self.app.repo.window_len().await;

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
                queue_size: queue_and_capacity.events_queue_size as u64,
                queue_capacity: queue_and_capacity.events_capacity_size as u64,
                queue_by_process_size: queue_and_capacity.process_queue_size as u64,
                queue_by_process_capacity: queue_and_capacity.process_queue_capacity as u64,
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
