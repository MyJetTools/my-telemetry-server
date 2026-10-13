use std::{collections::HashMap, sync::Arc};

use tokio::sync::Mutex;

use crate::settings::SettingsModel;

use super::{TelemetryGrpcSettings, TelemetryReaderGrpcClient};

/// One client per env: created with the first request to the env and reused after it.
pub struct TelemetryGrpcClients {
    clients: Mutex<HashMap<String, Arc<TelemetryReaderGrpcClient>>>,
}

impl TelemetryGrpcClients {
    pub fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// `None` - there is no such env in the settings.
    pub async fn get_or_create(
        &self,
        env: &str,
        settings: &Arc<SettingsModel>,
    ) -> Option<Arc<TelemetryReaderGrpcClient>> {
        let mut clients = self.clients.lock().await;

        if let Some(client) = clients.get(env) {
            return Some(client.clone());
        }

        let url = settings.get_env_url(env)?;

        let client =
            TelemetryReaderGrpcClient::new(Arc::new(TelemetryGrpcSettings::new(url.to_string())));
        client.set_ssh_private_key_resolver(settings.clone()).await;

        let client = Arc::new(client);
        clients.insert(env.to_string(), client.clone());

        Some(client)
    }
}
