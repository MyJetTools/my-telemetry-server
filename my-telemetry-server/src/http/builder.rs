use std::sync::Arc;

use my_http_server::controllers::ControllersMiddleware;

use crate::app_ctx::AppContext;

pub fn build_controllers(app: &Arc<AppContext>) -> ControllersMiddleware {
    let mut result = ControllersMiddleware::new(None, None);

    result.register_post_action(Arc::new(
        super::controllers::metric_event::PostMetricAction::new(app.clone()),
    ));

    result
}
