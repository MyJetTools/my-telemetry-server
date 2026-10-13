use std::{sync::Arc, time::Duration};

use rust_extensions::MyTimer;
use timers::{GcTimer, MetricsWriter, SaveStatisticsTimer, UploadToS3Timer};

mod app_ctx;
mod background;
mod caches;
mod db;
mod flows;
mod grpc_server;
mod http;
mod ignore_events;
mod mappers;
mod metrics_storage;
mod permanent_users;
mod process_id_user_id_links;
mod product_scope;
mod scripts;
mod settings;
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
    // `~/`: my-settings-reader takes the name as a path, and resolves only a leading `~`
    let settings_reader = crate::settings::SettingsReader::new("~/.my-telemetry").await;

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

    // Before anything which takes metrics is started: a writer signals it with the first
    // of them
    app.queue_executor
        .register(Arc::new(background::MetricsQueueJob::new(app.clone())));
    app.queue_executor.start();

    let mut http_server = http::start_up::setup_server(&app, http_port);

    self::flows::init(&app).await;

    let mut gc_timer = MyTimer::new_with_execute_timeout(
        Duration::from_secs(10),
        Duration::from_secs(60 * 5),
        my_logger::LOGGER.clone(),
    );

    gc_timer.register_timer("GcTimer", Arc::new(GcTimer::new(app.clone())));

    gc_timer.start();

    let mut save_statistics_timer =
        MyTimer::new(Duration::from_secs(1), my_logger::LOGGER.clone());

    save_statistics_timer.register_timer(
        "SaveStatisticsTimer",
        Arc::new(SaveStatisticsTimer::new(app.clone())),
    );

    save_statistics_timer.start();

    let mut metrics_writer_timer =
        MyTimer::new(Duration::from_millis(621), my_logger::LOGGER.clone());
    let metrics_writer: MetricsWriter = MetricsWriter::new(app.clone());
    metrics_writer_timer.register_timer("MetricsWriter", Arc::new(metrics_writer));

    metrics_writer_timer.start();

    if app.repo.has_s3() {
        // An upload of an hour may take minutes
        let mut s3_timer = MyTimer::new_with_execute_timeout(
            Duration::from_secs(60),
            Duration::from_secs(60 * 60),
            my_logger::LOGGER.clone(),
        );
        s3_timer.register_timer("UploadToS3Timer", Arc::new(UploadToS3Timer::new(app.clone())));
        s3_timer.start();
    }

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

    // The metrics of the last seconds are in memory on purpose: the storage holds each
    // second for a while before it writes it. On a clean shutdown there is no reason to
    // lose them, or the statistics they are counted into - or to leave an index to be
    // rebuilt.
    println!("Shutting down. Writing the metrics which wait in memory...");
    while crate::flows::take_in_queued_metrics(&app).await {}
    crate::flows::write_ready_metrics(&app, true).await;
    crate::flows::save_statistics(&app).await;
    println!("The metrics are written");
}
