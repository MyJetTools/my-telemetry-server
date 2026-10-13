use std::time::Duration;

use crate::db::MetricDto;
use crate::product_scope::{scope_of_writer, SCOPE_HEADER};
use crate::writer_grpc::telemetry_writer_server::TelemetryWriter;
use crate::writer_grpc::*;

const READ_TIMEOUT: Duration = Duration::from_secs(10);
use super::server::GrpcService;

#[tonic::async_trait]
impl TelemetryWriter for GrpcService {
    async fn upload(
        &self,
        request: tonic::Request<tonic::Streaming<TelemetryGrpcEvent>>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        // The product scope the writer names: the metrics go to the default one without it
        let scope = scope_of_writer(
            request
                .metadata()
                .get(SCOPE_HEADER)
                .and_then(|value| value.to_str().ok()),
        );

        let request = request.into_inner();

        let ignore_events = self.app.settings_reader.get_ignore_events().await;

        let events = my_grpc_extensions::read_grpc_stream::as_vec_with_transformation_and_filter(
            request,
            READ_TIMEOUT,
            &|grpc_model| {
                if ignore_events
                    .event_should_be_ignored(&grpc_model.service_name, &grpc_model.event_data)
                {
                    None
                } else {
                    Some(MetricDto::from_grpc_event(grpc_model, scope.clone()))
                }
            },
        )
        .await
        .unwrap();

        crate::flows::upload_events(&self.app, events);

        Ok(tonic::Response::new(()))
    }

    async fn ping(&self, _: tonic::Request<()>) -> Result<tonic::Response<()>, tonic::Status> {
        Ok(tonic::Response::new(()))
    }
}
