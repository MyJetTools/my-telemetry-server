use crate::{
    caches::{EventAmountsByHour, StatisticsByAppAndData},
    db::{HourAppDataStatisticsRepo, HourStatisticsRepo, PermanentMetricsRepo},
    permanent_users::PermanentUsersList,
    process_id_user_id_links::ProcessIdUserIdLinks,
    settings::SettingsReader,
    storage_by_hour::MetricsStorage,
    to_write_queue::ToWriteQueue,
};
use rust_extensions::AppStates;
use std::sync::Arc;
use tokio::sync::Mutex;

//pub const APP_NAME: &'static str = env!("CARGO_PKG_NAME");
pub const APP_VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub const METRICS_FILE_PREFIX: &'static str = "metrics";

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
    pub repo: MetricsStorage,

    pub permanent_metrics: PermanentMetricsRepo,

    pub settings_reader: Arc<SettingsReader>,
    pub to_write_queue: ToWriteQueue,
    pub cache: Mutex<StatisticsCache>,
    pub hour_statistics_repo: HourStatisticsRepo,
    pub hour_app_data_statistics_repo: HourAppDataStatisticsRepo,
}

impl AppContext {
    pub async fn new(settings_reader: Arc<SettingsReader>) -> AppContext {
        // Metrics no longer live in files named by a prefix - each hour is a folder under
        // the db path, named by its hour key.
        let metrics_path = settings_reader.get_db_path().await;

        let statistic_db_file_name = settings_reader
            .get_db_file_prefix("h_app_statistics.db")
            .await;

        let permanent_metrics_file_name = settings_reader
            .get_db_file_prefix("permanent_metrics.db")
            .await;

        let h_statistic_db_file_name = settings_reader.get_db_file_prefix("h_statistics.db").await;

        AppContext {
            permanent_metrics: PermanentMetricsRepo::new(permanent_metrics_file_name).await,
            to_write_queue: ToWriteQueue::new(),
            app_states: Arc::new(AppStates::create_initialized()),
            process_id: uuid::Uuid::new_v4().to_string(),
            repo: MetricsStorage::new(metrics_path),
            hour_app_data_statistics_repo: HourAppDataStatisticsRepo::new(statistic_db_file_name)
                .await,
            settings_reader,
            hour_statistics_repo: HourStatisticsRepo::new(h_statistic_db_file_name).await,
            cache: Mutex::new(StatisticsCache::new()),
        }
    }
}
