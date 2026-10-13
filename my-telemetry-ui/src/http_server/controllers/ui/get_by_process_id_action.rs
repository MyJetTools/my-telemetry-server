use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetByProcessId",
    controller: "ui",
    summary: "Events of the hour with the process id",
    description: "Events of the hour with the process id",
    input_data: "GetByProcessIdRequest",
    result: [
        {status_code: 200, description: "Events of the hour with the process id", model: "MetricsByProcessResponse"},
        {status_code: 404, description: "Env is not found"},
    ]
)]
pub struct GetByProcessIdAction {
    app: Arc<AppContext>,
}

impl GetByProcessIdAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetByProcessIdAction,
    input_data: GetByProcessIdRequest,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let metrics = crate::flows::get_by_process_id(
        &action.app,
        &input_data.env,
        input_data.hour_key,
        input_data.process_id,
    )
    .await?;

    HttpOutput::as_json(MetricsByProcessResponse { metrics }).into_ok_result(true)
}
