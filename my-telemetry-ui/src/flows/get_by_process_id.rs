use my_telemetry_ui_shared::MetricByProcessModel;

use crate::app::AppContext;

use super::FlowError;

/// Every event of the hour with the process id, in every product scope.
pub async fn get_by_process_id(
    app: &AppContext,
    env: &str,
    hour_key: i64,
    process_id: i64,
) -> Result<Vec<MetricByProcessModel>, FlowError> {
    let grpc_client = crate::scripts::get_grpc_client(app, env).await?;

    let events = grpc_client
        .get_by_process_id(crate::reader_grpc::GetByProcessIdRequest {
            process_id,
            hour_key,
        })
        .await?
        .into_vec()
        .await?;

    Ok(events)
}
