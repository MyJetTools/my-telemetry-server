use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetAvailableHours",
    controller: "ui",
    summary: "Hours the product scope of the env has metrics of",
    description: "Hours the product scope of the env has metrics of",
    input_data: "GetAvailableHoursHttpInput",
    result: [
        {status_code: 200, description: "Hours the product scope of the env has metrics of", model: "GetAvailableHoursResponse"},
        {status_code: 400, description: "The product scope is not a name of one"},
        {status_code: 404, description: "Env is not found"},
    ]
)]
pub struct GetAvailableHoursAction {
    app: Arc<AppContext>,
}

impl GetAvailableHoursAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetAvailableHoursAction,
    input_data: GetAvailableHoursHttpInput,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let hours =
        crate::flows::get_available_hours(&action.app, &input_data.env, input_data.scope).await?;
    HttpOutput::as_json(GetAvailableHoursResponse { hours }).into_ok_result(true)
}
