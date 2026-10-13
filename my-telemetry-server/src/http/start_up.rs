use std::{net::SocketAddr, sync::Arc};

use my_http_server::controllers::swagger::SwaggerMiddleware;
use my_http_server::MyHttpServer;

use crate::app_ctx::AppContext;

pub fn setup_server(app: &Arc<AppContext>, port: u16) -> MyHttpServer {
    let mut http_server = MyHttpServer::new(SocketAddr::from(([0, 0, 0, 0], port)));
    println!("Http server port is: {}", port);
    let controllers = Arc::new(super::builder::build_controllers(&app));

    let swagger_middleware = SwaggerMiddleware::new(
        controllers.clone(),
        "MyTelemetry".to_string(),
        crate::app_ctx::APP_VERSION.to_string(),
    );

    http_server.add_middleware(Arc::new(swagger_middleware));
    http_server.add_middleware(controllers);

    // No UI here: it is my-telemetry-ui, which reads this server over gRPC
    // (`TelemetryReader`). HTTP is for writing metrics only.
    http_server
}
