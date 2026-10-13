use my_telemetry_ui_shared::{GetServiceMetricsOverview, ServiceOverviewContract};

use crate::{app::AppContext, reader_grpc::GetByAppRequest};

use super::FlowError;

/// The actions of a service of the product scope in the hour, with their durations and
/// outcomes.
pub async fn get_service_overview(
    app: &AppContext,
    request: GetServiceMetricsOverview,
) -> Result<Vec<ServiceOverviewContract>, FlowError> {
    let grpc_client = crate::scripts::get_grpc_client(app, &request.env).await?;

    let actions = grpc_client
        .get_app_actions(GetByAppRequest {
            app_id: request.id,
            hour_key: request.hour_key,
            product_scope: request.scope,
        })
        .await?
        .into_vec()
        .await?;

    Ok(actions)
}
