use crate::app_ctx::AppContext;
use crate::reader_grpc::telemetry_reader_server::TelemetryReaderServer;
use crate::writer_grpc::telemetry_writer_server::TelemetryWriterServer;

use std::net::SocketAddr;
use std::sync::Arc;
use tonic::transport::Server;

#[derive(Clone)]

pub struct GrpcService {
    pub app: Arc<AppContext>,
}

impl GrpcService {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

pub fn start(app: &Arc<AppContext>, port: u16) {
    let app = app.clone();
    tokio::spawn(start_server(app, port));
}

async fn start_server(app: Arc<AppContext>, port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let service = GrpcService::new(app);

    println!("Listening to {:?} as grpc endpoint", addr);

    anyhow::Context::context(
        Server::builder()
            .add_service(TelemetryWriterServer::new(service.clone()))
            .add_service(TelemetryReaderServer::new(service.clone()))
            .serve(addr)
            .await,
        "Server error",
    )
    .unwrap();
}
