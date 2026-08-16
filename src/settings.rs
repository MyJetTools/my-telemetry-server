use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ignore_events::IgnoreEvents;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IgnoreEvent {
    pub name: String,
    pub data: String,
}

#[derive(my_settings_reader::SettingsModel, Serialize, Deserialize, Debug, Clone)]
pub struct SettingsModel {
    #[serde(rename = "DbPath")]
    pub db_path: String,
    #[serde(rename = "HoursToGc")]
    pub hours_to_gc: i64,
    #[serde(rename = "IgnoreEvents")]
    pub ignore_events: Vec<IgnoreEvent>,

    #[serde(rename = "SecondsToFlush")]
    pub seconds_to_flush: Option<i64>,

    /// How long a metric stays in the in-memory window before it reaches disk. This is
    /// what keeps the data file append-only: anything arriving inside this many seconds of
    /// its own timestamp is absorbed by an in-memory sort. Anything later costs a rewrite
    /// of the file tail.
    ///
    /// It is also exactly what a crash loses.
    #[serde(rename = "WindowSeconds")]
    pub window_seconds: Option<i64>,
}

impl SettingsReader {
    pub async fn get_ignore_events(&self) -> IgnoreEvents {
        let read_access = self.settings.read().await;

        IgnoreEvents::new(read_access.ignore_events.clone())
    }

    pub async fn get_db_path(&self) -> String {
        let read_access = self.settings.read().await;

        let db_path = if read_access.db_path.ends_with(std::path::MAIN_SEPARATOR) {
            &read_access.db_path[..read_access.db_path.len() - 1]
        } else {
            read_access.db_path.as_str()
        };

        rust_extensions::file_utils::format_path(db_path).to_string()
    }

    pub async fn get_db_file_prefix(&self, file_name: &str) -> String {
        let read_access = self.settings.read().await;

        let mut result = if read_access.db_path.starts_with("~") {
            read_access
                .db_path
                .replace("~", &std::env::var("HOME").unwrap())
        } else {
            read_access.db_path.clone()
        };

        if !result.ends_with(std::path::MAIN_SEPARATOR) {
            result.push(std::path::MAIN_SEPARATOR)
        }

        result.push_str(file_name);

        result
    }

    pub async fn get_hours_to_gc(&self) -> Duration {
        let read_access = self.settings.read().await;
        Duration::from_secs((read_access.hours_to_gc * 3600) as u64)
    }

    pub async fn get_seconds_to_flush(&self) -> i64 {
        let read_access = self.settings.read().await;
        read_access.seconds_to_flush.unwrap_or(3)
    }

    pub async fn get_window_seconds(&self) -> i64 {
        let read_access = self.settings.read().await;
        read_access.window_seconds.unwrap_or(60)
    }
}
