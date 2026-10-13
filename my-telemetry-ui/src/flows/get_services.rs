use my_telemetry_ui_shared::ServiceHttpModel;

use crate::{app::AppContext, reader_grpc::GetAppsRequest};

use super::FlowError;

/// The services which wrote metrics of the product scope in the hour: how many and how
/// long on average.
pub async fn get_services(
    app: &AppContext,
    env: &str,
    scope: Option<String>,
    hour_key: i64,
) -> Result<Vec<ServiceHttpModel>, FlowError> {
    let grpc_client = crate::scripts::get_grpc_client(app, env).await?;

    let services = grpc_client
        .get_apps(GetAppsRequest {
            hour_key,
            product_scope: scope,
        })
        .await?
        .into_vec()
        .await?;

    Ok(services)
}
