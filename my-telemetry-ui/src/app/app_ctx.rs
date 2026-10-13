use std::sync::Arc;

use my_settings_reader::SettingsReader;
use rust_extensions::AppStates;

use crate::{grpc_client::TelemetryGrpcClients, settings::SettingsModel};

pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const SETTINGS_FILE_NAME: &str = "~/.my-telemetry-ui";

pub struct AppContext {
    pub telemetry_grpc_clients: TelemetryGrpcClients,
    pub states: Arc<AppStates>,
    pub settings_reader: SettingsReader<SettingsModel>,
}

impl AppContext {
    pub fn new() -> Self {
        Self {
            telemetry_grpc_clients: TelemetryGrpcClients::new(),
            states: Arc::new(AppStates::create_initialized()),
            settings_reader: SettingsReader::new(SETTINGS_FILE_NAME),
        }
    }
}
