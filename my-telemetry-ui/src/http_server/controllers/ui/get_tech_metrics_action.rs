use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetTechMetrics",
    controller: "ui",
    summary: "Queue and cache sizes of the env",
    description: "Queue and cache sizes of the env",
    input_data: "EnvHttpRequest",
    result: [
        {status_code: 200, description: "Queue and cache sizes of the env", model: "TechMetricsHttpModel"},
        {status_code: 404, description: "Env is not found"},
    ]
)]
pub struct GetTechMetricsAction {
    app: Arc<AppContext>,
}

impl GetTechMetricsAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetTechMetricsAction,
    input_data: EnvHttpRequest,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let result = crate::flows::get_tech_metrics(&action.app, &input_data.env).await?;
    HttpOutput::as_json(result).into_ok_result(true)
}
