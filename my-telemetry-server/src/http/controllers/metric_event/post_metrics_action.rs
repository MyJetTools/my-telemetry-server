use std::sync::Arc;

use my_http_server::{HttpContext, HttpFailResult, HttpOkResult, HttpOutput, HttpRequestHeaders};

use crate::{
    app_ctx::AppContext,
    product_scope::{scope_of_writer, SCOPE_HEADER},
};

use super::models::NewMetricsEvent;

#[my_http_server::macros::http_route(
    method: "POST",
    route: "/api/add",
    controller: "api",
    description: "New Metric Event",
    summary: "New Metric Event",
    input_data: "NewMetricsEvent",

)]
pub struct PostMetricAction {
    app: Arc<AppContext>,
}

impl PostMetricAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}
async fn handle_request(
    action: &PostMetricAction,
    input_data: NewMetricsEvent,
    ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let ignore_events = action.app.settings_reader.get_ignore_events().await;

    // The product scope the writer names: the metrics go to the default one without it
    let scope = scope_of_writer(
        ctx.request
            .get_headers()
            .try_get_case_insensitive_as_str(SCOPE_HEADER)
            .ok()
            .flatten(),
    );

    let dto = input_data.into_dto(&ignore_events, &scope)?;

    crate::flows::upload_events(&action.app, dto);

    return HttpOutput::Empty.into_ok_result(true).into();
}
