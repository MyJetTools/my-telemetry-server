use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetByServiceData",
    controller: "ui",
    summary: "Last events of an action of a service",
    description: "Last events of an action of a service",
    input_data: "GetByServiceDataRequest",
    result: [
        {status_code: 200, description: "Last events of an action of a service", model: "MetricsResponse"},
        {status_code: 400, description: "The product scope is not a name of one"},
        {status_code: 404, description: "Env is not found"},
    ]
)]
pub struct GetByServiceDataAction {
    app: Arc<AppContext>,
}

impl GetByServiceDataAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetByServiceDataAction,
    input_data: GetByServiceDataRequest,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let metrics = crate::flows::get_by_service_data(&action.app, input_data).await?;
    HttpOutput::as_json(MetricsResponse { metrics }).into_ok_result(true)
}
