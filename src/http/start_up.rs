use std::{net::SocketAddr, sync::Arc};

use my_http_server::controllers::swagger::SwaggerMiddleware;
use my_http_server::{MyHttpServer, StaticFilesMiddleware};

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

    // The UI in `ui/` is a Dioxus wasm client compiled into `wwwroot/`. Serving
    // `index.html` both as the index and as the not-found file is what makes its
    // client-side routes (`/actions/...`, `/process/...`) survive a page reload:
    // the server hands back the app and the router resolves the path in browser.
    http_server.add_middleware(Arc::new(
        StaticFilesMiddleware::new()
            .add_index_file("index.html")
            .set_not_found_file("index.html".to_string()),
    ));

    http_server
}
