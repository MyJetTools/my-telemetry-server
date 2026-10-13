use std::sync::Arc;

mod app;
mod flows;
mod grpc_client;
mod http_server;
mod mappers;
mod scripts;
mod settings;

pub mod reader_grpc {
    tonic::include_proto!("reader");
}

#[tokio::main]
async fn main() {
    let app = Arc::new(app::AppContext::new());

    // A missing or broken settings file stops the app here - not on the first request
    app.settings_reader.get_settings().await;

    crate::http_server::start(&app);

    app.states.wait_until_shutdown().await;
}
