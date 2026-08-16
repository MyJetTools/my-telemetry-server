use std::{sync::Arc, time::Duration};

use rust_extensions::MyTimer;
use timers::{GcTimer, MetricsWriter, SaveStatisticsTimer};

mod app_ctx;
mod caches;
mod db;
mod flows;
mod grpc_server;
mod http;
mod ignore_events;
mod lazy_lock;
mod mappers;
mod metric_file;
mod permanent_users;
mod process_id_user_id_links;
mod scripts;
mod settings;
mod storage_by_hour;
mod timers;
mod to_write_queue;

pub mod writer_grpc {
    tonic::include_proto!("writer");
}

pub mod reader_grpc {
    tonic::include_proto!("reader");
}

const DEFAULT_HTTP_PORT: u16 = 8000;
const DEFAULT_GRPC_PORT: u16 = 8888;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() {
    let settings_reader = crate::settings::SettingsReader::new(".my-telemetry").await;

    let settings_reader = Arc::new(settings_reader);

    let app = app_ctx::AppContext::new(settings_reader).await;

    let app = Arc::new(app);

    let http_port = if let Ok(result) = std::env::var("HTTP_PORT") {
        match result.parse() {
            Ok(port) => port,
            Err(_) => DEFAULT_HTTP_PORT,
        }
    } else {
        DEFAULT_HTTP_PORT
    };

    let mut http_server = http::start_up::setup_server(&app, http_port);

    self::flows::init(&app).await;

    let mut gc_timer =
        MyTimer::new_with_execute_timeout(Duration::from_secs(10), Duration::from_secs(60 * 5));

    gc_timer.register_timer("GcTimer", Arc::new(GcTimer::new(app.clone())));

    gc_timer.start(app.app_states.clone(), my_logger::LOGGER.clone());

    let mut save_statistics_timer = MyTimer::new(Duration::from_secs(1));

    save_statistics_timer.register_timer(
        "SaveStatisticsTimer",
        Arc::new(SaveStatisticsTimer::new(app.clone())),
    );

    save_statistics_timer.start(app.app_states.clone(), my_logger::LOGGER.clone());

    let mut metrics_writer_timer = MyTimer::new(Duration::from_millis(621));
    let metrics_writer: MetricsWriter = MetricsWriter::new(app.clone());
    metrics_writer_timer.register_timer("MetricsWriter", Arc::new(metrics_writer));

    metrics_writer_timer.start(app.app_states.clone(), my_logger::LOGGER.clone());

    http_server.start(app.app_states.clone(), my_logger::LOGGER.clone());

    let grpc_port = if let Ok(result) = std::env::var("GRPC_PORT") {
        match result.parse() {
            Ok(port) => port,
            Err(_) => DEFAULT_GRPC_PORT,
        }
    } else {
        DEFAULT_GRPC_PORT
    };

    grpc_server::start(&app, grpc_port);
    app.app_states.wait_until_shutdown().await;

    // The storage holds the last window's worth of metrics in memory on purpose. On a
    // clean shutdown there is no reason to lose it.
    println!("Shutting down. Flushing the metrics window to disk...");
    app.repo.flush_all().await;
    println!("Metrics window is flushed");
}
