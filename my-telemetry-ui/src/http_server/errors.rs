use my_grpc_extensions::GrpcReadError;
use my_http_server::HttpFailResult;

use crate::flows::FlowError;

impl From<FlowError> for HttpFailResult {
    fn from(err: FlowError) -> Self {
        match err {
            FlowError::EnvNotFound(env) => {
                HttpFailResult::as_not_found(format!("Env '{}' is not found", env), false)
            }
            // What the request asks for is not what my-telemetry-server takes - a product
            // scope which is not a name of one: the mistake of the caller, not a failure
            FlowError::Grpc(GrpcReadError::TonicStatus(status))
                if status.code() == tonic::Code::InvalidArgument =>
            {
                HttpFailResult::as_validation_error(status.message().to_string())
            }
            FlowError::Grpc(err) => HttpFailResult::as_fatal_error(format!(
                "Request to my-telemetry-server failed: {:?}",
                err
            )),
        }
    }
}
