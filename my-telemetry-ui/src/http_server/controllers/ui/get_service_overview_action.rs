use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetServiceOverview",
    controller: "ui",
    summary: "Actions of a service in the hour",
    description: "Actions of a service in the hour",
    input_data: "GetServiceMetricsOverview",
    result: [
        {status_code: 200, description: "Actions of a service in the hour", model: "GetServiceOverviewResponse"},
        {status_code: 400, description: "The product scope is not a name of one"},
        {status_code: 404, description: "Env is not found"},
    ]
)]
pub struct GetServiceOverviewAction {
    app: Arc<AppContext>,
}

impl GetServiceOverviewAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetServiceOverviewAction,
    input_data: GetServiceMetricsOverview,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let data = crate::flows::get_service_overview(&action.app, input_data).await?;

    HttpOutput::as_json(GetServiceOverviewResponse { data }).into_ok_result(true)
}
