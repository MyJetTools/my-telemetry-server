use std::sync::Arc;

use my_http_server::controllers::ControllersMiddleware;

use crate::app::AppContext;

use super::controllers::*;

pub fn build_controllers(app: &Arc<AppContext>) -> ControllersMiddleware {
    let mut result = ControllersMiddleware::new(None, None);

    result.register_get_action(Arc::new(ui::GetAppInfoAction::new(app.clone())));
    result.register_get_action(Arc::new(ui::GetProductScopesAction::new(app.clone())));
    result.register_get_action(Arc::new(ui::GetAvailableHoursAction::new(app.clone())));
    result.register_get_action(Arc::new(ui::GetServicesAction::new(app.clone())));
    result.register_get_action(Arc::new(ui::GetServiceOverviewAction::new(app.clone())));
    result.register_get_action(Arc::new(ui::GetByServiceDataAction::new(app.clone())));
    result.register_get_action(Arc::new(ui::GetByProcessIdAction::new(app.clone())));
    result.register_get_action(Arc::new(ui::GetTechMetricsAction::new(app.clone())));

    result
}
