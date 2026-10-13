use std::sync::Arc;

use my_http_server::macros::*;
use my_http_server::*;
use my_telemetry_ui_shared::*;

use crate::app::AppContext;

#[http_route(
    method: "GET",
    route: "/ui/GetProductScopes",
    controller: "ui",
    summary: "Product scopes the env has metrics of",
    description: "Product scopes the env has metrics of",
    input_data: "EnvHttpRequest",
    result: [
        {status_code: 200, description: "Product scopes the env has metrics of", model: "ProductScopesHttpModel"},
        {status_code: 404, description: "Env is not found"},
    ]
)]
pub struct GetProductScopesAction {
    app: Arc<AppContext>,
}

impl GetProductScopesAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetProductScopesAction,
    input_data: EnvHttpRequest,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let result = crate::flows::get_product_scopes(&action.app, &input_data.env).await?;
    HttpOutput::as_json(result).into_ok_result(true)
}
