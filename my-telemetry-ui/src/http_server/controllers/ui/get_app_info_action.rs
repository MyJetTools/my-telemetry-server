use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetAppInfo",
    controller: "ui",
    summary: "The version of the UI and the envs it reads",
    description: "The version of the UI and the envs it reads",
    result: [
        {status_code: 200, description: "The version of the UI and the envs it reads", model: "AppInfoHttpModel"},
    ]
)]
pub struct GetAppInfoAction {
    app: Arc<AppContext>,
}

impl GetAppInfoAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetAppInfoAction,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let result = crate::flows::get_app_info(&action.app).await;
    HttpOutput::as_json(result).into_ok_result(true)
}
