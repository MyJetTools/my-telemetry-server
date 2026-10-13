use my_grpc_extensions::{GrpcClientSettings, GrpcUrl};

use super::TelemetryReaderGrpcClient;

/// Url of the my-telemetry-server of one env - `ssh:...->http://...` when it is reached
/// over ssh.
pub struct TelemetryGrpcSettings {
    url: String,
}

impl TelemetryGrpcSettings {
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[async_trait::async_trait]
impl GrpcClientSettings for TelemetryGrpcSettings {
    async fn get_grpc_url(&self, name: &'static str) -> GrpcUrl {
        if name == TelemetryReaderGrpcClient::get_service_name() {
            return self.url.clone().into();
        }

        panic!("Unknown grpc service name: {}", name);
    }
}
