use my_grpc_extensions::GrpcReadError;

pub enum FlowError {
    EnvNotFound(String),
    Grpc(GrpcReadError),
}

impl From<GrpcReadError> for FlowError {
    fn from(err: GrpcReadError) -> Self {
        Self::Grpc(err)
    }
}
