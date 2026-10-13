use my_telemetry_ui_shared::{GetByServiceDataRequest, MetricHttpModel};

use crate::{app::AppContext, reader_grpc::GetAppEventsByActionRequest};

use super::FlowError;

/// The last events of an action of a service of the product scope in the hour.
pub async fn get_by_service_data(
    app: &AppContext,
    request: GetByServiceDataRequest,
) -> Result<Vec<MetricHttpModel>, FlowError> {
    let grpc_client = crate::scripts::get_grpc_client(app, &request.env).await?;

    let events = grpc_client
        .get_app_events_by_action(GetAppEventsByActionRequest {
            app_id: request.id,
            data: request.data,
            hour_key: request.hour_key,
            // An empty client id is "no filter" on the gRPC side
            client_id: request.client_id.unwrap_or_default(),
            from_sec_within_hour: request.from_second_within_hour,
            product_scope: request.scope,
        })
        .await?
        .into_vec()
        .await?;

    Ok(events)
}
