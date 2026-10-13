use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetServices",
    controller: "ui",
    summary: "Services of the hour",
    description: "Services of the hour",
    input_data: "GetServicesHttpInput",
    result: [
        {status_code: 200, description: "Services of the hour", model: "GetServicesResponse"},
        {status_code: 400, description: "The product scope is not a name of one"},
        {status_code: 404, description: "Env is not found"},
    ]
)]
pub struct GetServicesAction {
    app: Arc<AppContext>,
}

impl GetServicesAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetServicesAction,
    input_data: GetServicesHttpInput,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let services = crate::flows::get_services(
        &action.app,
        &input_data.env,
        input_data.scope,
        input_data.hour_key,
    )
    .await?;
    HttpOutput::as_json(GetServicesResponse { services }).into_ok_result(true)
}
