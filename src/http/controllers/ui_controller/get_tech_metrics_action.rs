use std::sync::Arc;

use my_http_server::{HttpContext, HttpFailResult, HttpOkResult, HttpOutput};

use crate::app_ctx::AppContext;

use super::models::*;

#[my_http_server::macros::http_route(
    method: "GET",
    route: "/ui/GetTechMetrics",
    controller: "ui",
    description: "Internal queue and cache sizes",
    summary: "Internal queue and cache sizes",
    result:[
        {status_code: 200, description: "Tech metrics", model="TechMetricsHttpModel"},
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
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let queue_and_capacity = action
        .app
        .to_write_queue
        .get_queue_and_capacity_and_by_process_capacity()
        .await;

    let storage_window_size = action.app.repo.window_len().await;

    let cache_read_access = action.app.cache.lock().await;

    let app_data_size = cache_read_access
        .statistics_by_app_and_data
        .get_size_and_capacity();

    let app_data_hour_size = cache_read_access
        .statistics_by_app_and_data
        .get_queue_hours_size();

    let (user_id_links_size, user_id_links_capacity) = cache_read_access
        .process_id_user_id_links
        .get_size_and_capacity();

    let result = TechMetricsHttpModel {
        app_data_hours_size: app_data_hour_size.0 as u64,
        app_data_to_persist_hours_size: app_data_hour_size.1 as u64,
        queue_size: queue_and_capacity.events_queue_size as u64,
        queue_capacity: queue_and_capacity.events_capacity_size as u64,
        queue_by_process_size: queue_and_capacity.process_queue_size as u64,
        queue_by_process_capacity: queue_and_capacity.process_queue_capacity as u64,
        user_id_links_size: user_id_links_size as u64,
        user_id_links_capacity: user_id_links_capacity as u64,
        app_data_size: app_data_size.0 as u64,
        app_data_capacity: app_data_size.1 as u64,
        storage_window_size: storage_window_size as u64,
    };

    HttpOutput::as_json(result).into_ok_result(true).into()
}
