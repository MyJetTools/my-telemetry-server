use std::sync::Arc;

use crate::{app::AppContext, flows::FlowError, grpc_client::TelemetryReaderGrpcClient};

pub async fn get_grpc_client(
    app: &AppContext,
    env: &str,
) -> Result<Arc<TelemetryReaderGrpcClient>, FlowError> {
    let settings = app.settings_reader.get_settings().await;

    app.telemetry_grpc_clients
        .get_or_create(env, &settings)
        .await
        .ok_or_else(|| FlowError::EnvNotFound(env.to_string()))
}
