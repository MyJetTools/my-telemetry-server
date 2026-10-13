use my_telemetry_ui_shared::TechMetricsHttpModel;

use crate::app::AppContext;

use super::FlowError;

/// The queue and cache sizes of the env's my-telemetry-server.
pub async fn get_tech_metrics(
    app: &AppContext,
    env: &str,
) -> Result<TechMetricsHttpModel, FlowError> {
    let grpc_client = crate::scripts::get_grpc_client(app, env).await?;

    let tech_metrics = grpc_client.get_tech_metrics(()).await?;

    Ok(tech_metrics.into())
}
