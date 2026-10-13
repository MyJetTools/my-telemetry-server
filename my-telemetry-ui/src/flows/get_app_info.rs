use my_telemetry_ui_shared::AppInfoHttpModel;

use crate::app::{AppContext, APP_VERSION};

pub async fn get_app_info(app: &AppContext) -> AppInfoHttpModel {
    let settings = app.settings_reader.get_settings().await;

    AppInfoHttpModel {
        version: APP_VERSION.to_string(),
        envs: settings.get_envs(),
    }
}
