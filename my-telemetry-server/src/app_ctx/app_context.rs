use crate::{
    caches::{EventAmountsByHour, StatisticsByAppAndData},
    db::{HourAppDataStatisticsRepo, HourStatisticsRepo, PermanentMetricsRepo},
    permanent_users::PermanentUsersList,
    process_id_user_id_links::ProcessIdUserIdLinks,
    settings::SettingsReader,
    metrics_storage::{MetricsStorage, S3Archive},
    to_write_queue::ToWriteQueue,
};
use rust_extensions::{background_executor::BackgroundExecutor, AppStates};
use std::sync::Arc;
use tokio::sync::Mutex;

//pub const APP_NAME: &'static str = env!("CARGO_PKG_NAME");
pub const APP_VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub struct StatisticsCache {
    pub event_amount_by_hours: EventAmountsByHour,
    pub statistics_by_app_and_data: StatisticsByAppAndData,
    pub process_id_user_id_links: ProcessIdUserIdLinks,
    pub permanent_users_list: PermanentUsersList,
}

impl StatisticsCache {
    pub fn new() -> Self {
        Self {
            statistics_by_app_and_data: StatisticsByAppAndData::new(),
            event_amount_by_hours: EventAmountsByHour::new(),
            process_id_user_id_links: ProcessIdUserIdLinks::new(),
            permanent_users_list: PermanentUsersList::new(),
        }
    }
}

pub struct AppContext {
    pub app_states: Arc<AppStates>,
    pub process_id: String,
    pub repo: Arc<MetricsStorage>,

    pub permanent_metrics: PermanentMetricsRepo,

    pub settings_reader: Arc<SettingsReader>,
    /// What the writers sent, until it is taken in - at once: a writer signals
    /// `queue_executor` as it puts metrics here
    pub to_write_queue: ToWriteQueue,
    pub queue_executor: BackgroundExecutor,
    /// One pass over the queue at a time. `tokio::sync::Mutex`: it is held across the
    /// lock of the statistics.
    pub queue_pass: Mutex<()>,
    pub cache: Mutex<StatisticsCache>,
    pub hour_statistics_repo: HourStatisticsRepo,
    pub hour_app_data_statistics_repo: HourAppDataStatisticsRepo,
}

impl AppContext {
    pub async fn new(settings_reader: Arc<SettingsReader>) -> AppContext {
        // The metrics are folders under the db path: a product scope, an hour in it.
        let metrics_path = settings_reader.get_db_path().await;

        // The statistics are kept by product scope: other tables than the ones before it,
        // so other files - the old ones are not read.
        let statistic_db_file_name = settings_reader
            .get_db_file_prefix("h_app_statistics_by_scope.db")
            .await;

        let permanent_metrics_file_name = settings_reader
            .get_db_file_prefix("permanent_metrics.db")
            .await;

        let h_statistic_db_file_name = settings_reader
            .get_db_file_prefix("h_statistics_by_scope.db")
            .await;

        // Where the hours go once their time on the local disk is over; without it they
        // are deleted then. A connection string which does not read stops the start.
        let s3 = settings_reader
            .get_s3_connection()
            .await
            .map(|settings| S3Archive::new(&settings));

        AppContext {
            permanent_metrics: PermanentMetricsRepo::new(permanent_metrics_file_name).await,
            to_write_queue: ToWriteQueue::new(),
            queue_executor: BackgroundExecutor::new("MetricsQueue", my_logger::LOGGER.clone()),
            queue_pass: Mutex::new(()),
            app_states: Arc::new(AppStates::create_initialized()),
            process_id: uuid::Uuid::new_v4().to_string(),
            repo: Arc::new(MetricsStorage::new(metrics_path, s3)),
            hour_app_data_statistics_repo: HourAppDataStatisticsRepo::new(statistic_db_file_name)
                .await,
            settings_reader,
            hour_statistics_repo: HourStatisticsRepo::new(h_statistic_db_file_name).await,
            cache: Mutex::new(StatisticsCache::new()),
        }
    }
}
