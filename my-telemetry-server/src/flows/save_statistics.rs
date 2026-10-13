use crate::app_ctx::AppContext;

/// Writes the statistics which have changed since they were written the last time
pub async fn save_statistics(app: &AppContext) {
    let (app_data_metrics, app_hour_metrics) = {
        let mut metrics_access = app.cache.lock().await;

        let app_data_metrics = metrics_access.statistics_by_app_and_data.get_to_persist();

        let app_hour_metrics = metrics_access.event_amount_by_hours.get_to_persist();

        (app_data_metrics, app_hour_metrics)
    };

    if let Some(app_data_metrics) = app_data_metrics {
        crate::scripts::write_hour_app_data_statistics(app, app_data_metrics).await;
    }

    if let Some(app_hour_metrics) = app_hour_metrics {
        crate::scripts::write_hour_statistics_to_db(app, app_hour_metrics).await;
    }
}
