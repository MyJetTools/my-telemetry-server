use serde::{Deserialize, Serialize};

use crate::ignore_events::IgnoreEvents;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IgnoreEvent {
    pub name: String,
    pub data: String,
}

/// Where the hours go two hours after they end, to be kept for `HoursToGc`. Without it
/// they are deleted then.
///
/// One connection string, the `Key=Value;Key=Value` shape of my-logger-server:
///
/// ```text
/// S3ConnString: "Endpoint=https://fsn1.your-objectstorage.com;Region=fsn1;AccessKey=...;SecretKey=...;Bucket=my-telemetry;Prefix=prod-1"
/// ```
#[derive(Debug, Clone)]
pub struct S3ConnectionSettings {
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    /// The folder of the hours in the bucket. It has no default: there are several
    /// my-telemetry-servers, and two which shared a folder would overwrite - and
    /// garbage-collect - the hours of each other.
    pub prefix: String,
    /// `Debug=1` - trace every S3 request to the console. Off unless asked for.
    pub debug: bool,
}

impl S3ConnectionSettings {
    /// A missing or misspelled key fails the start rather than silently leaving S3 out
    pub fn parse(conn_string: &str) -> Self {
        let mut endpoint = None;
        let mut region = None;
        let mut access_key = None;
        let mut secret_key = None;
        let mut bucket = None;
        let mut prefix = None;
        let mut debug = None;

        for pair in conn_string.split(';') {
            let pair = pair.trim();

            if pair.is_empty() {
                continue;
            }

            // Only the first `=` separates - a base64 secret key carries its own
            let Some(separator) = pair.find('=') else {
                panic!("Invalid S3ConnString: '{}' is not Key=Value", pair);
            };

            let key = pair[..separator].trim();
            let value = pair[separator + 1..].trim().to_string();

            match key {
                "Endpoint" => endpoint = Some(value),
                "Region" => region = Some(value),
                "AccessKey" => access_key = Some(value),
                "SecretKey" => secret_key = Some(value),
                "Bucket" => bucket = Some(value),
                "Prefix" => prefix = Some(value),
                "Debug" => debug = Some(parse_bool(value.as_str(), "Debug")),
                _ => panic!(
                    "Invalid S3ConnString: unknown key '{}'. Expected Endpoint, Region, AccessKey, SecretKey, Bucket, Prefix, and optionally Debug",
                    key
                ),
            }
        }

        let prefix = required(prefix, "Prefix");
        let prefix = prefix.trim_matches('/');

        if prefix.is_empty() {
            panic!("Invalid S3ConnString: 'Prefix' is missing");
        }

        Self {
            endpoint: required(endpoint, "Endpoint"),
            region: required(region, "Region"),
            access_key: required(access_key, "AccessKey"),
            secret_key: required(secret_key, "SecretKey"),
            bucket: required(bucket, "Bucket"),
            prefix: prefix.to_string(),
            debug: debug.unwrap_or(false),
        }
    }
}

/// Spelled out rather than `== "1"`: a `Debug=true` silently meaning "off" is worse than a
/// refusal to start
fn parse_bool(value: &str, key: &str) -> bool {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => panic!(
            "Invalid S3ConnString: '{}' expects 1/0, true/false, yes/no or on/off - got '{}'",
            key, value
        ),
    }
}

fn required(value: Option<String>, key: &str) -> String {
    match value {
        Some(value) if !value.is_empty() => value,
        _ => panic!("Invalid S3ConnString: '{}' is missing", key),
    }
}

#[derive(my_settings_reader::SettingsModel, Serialize, Deserialize, Debug, Clone)]
pub struct SettingsModel {
    #[serde(rename = "DbPath")]
    pub db_path: String,
    #[serde(rename = "HoursToGc")]
    pub hours_to_gc: i64,
    #[serde(rename = "IgnoreEvents")]
    pub ignore_events: Vec<IgnoreEvent>,

    /// How long the metrics of a second wait in memory, from the first of them, before they
    /// are written: what comes for the second within that is written with it, in one block.
    /// What comes later is merged into the block - cheap, but a write of its own.
    ///
    /// It is the time the `client_id` of a process has to come in: it comes with one of
    /// the events of the process, and the ones written before it are left without. And it
    /// is what a crash loses. 5 when it is not set, as in my-logger-server.
    #[serde(rename = "WindowSeconds")]
    pub window_seconds: Option<i64>,

    /// See [`S3ConnectionSettings`]
    #[serde(rename = "S3ConnString")]
    pub s3_conn_string: Option<String>,
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

    /// `None` - no S3: the hours are deleted two hours after they end
    pub async fn get_s3_connection(&self) -> Option<S3ConnectionSettings> {
        let read_access = self.settings.read().await;
        let conn_string = read_access.s3_conn_string.as_deref()?.trim();

        if conn_string.is_empty() {
            return None;
        }

        Some(S3ConnectionSettings::parse(conn_string))
    }

    /// How many hours the metrics are kept in S3. Without S3 it is not used: an hour is
    /// deleted from the local disk two hours after it ends.
    pub async fn get_hours_to_gc(&self) -> u64 {
        let read_access = self.settings.read().await;
        read_access.hours_to_gc.max(0) as u64
    }

    pub async fn get_window_seconds(&self) -> u64 {
        let read_access = self.settings.read().await;
        read_access.window_seconds.unwrap_or(5).max(0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_connection_string() {
        let parsed = S3ConnectionSettings::parse(
            "Endpoint=https://s3.eu-central-1.amazonaws.com;Region=eu-central-1;AccessKey=AKIA123;SecretKey=abc/def+ghi=;Bucket=my-bucket;Prefix=/prod-1/",
        );

        assert_eq!("https://s3.eu-central-1.amazonaws.com", parsed.endpoint);
        assert_eq!("eu-central-1", parsed.region);
        assert_eq!("AKIA123", parsed.access_key);
        // A base64 secret carries its own '=' - only the first one separates
        assert_eq!("abc/def+ghi=", parsed.secret_key);
        assert_eq!("my-bucket", parsed.bucket);
        // A folder, with no slashes around it
        assert_eq!("prod-1", parsed.prefix);
        assert!(!parsed.debug);
    }

    #[test]
    fn debug_is_optional() {
        let parsed = S3ConnectionSettings::parse(
            "Endpoint=https://s3;Region=eu;AccessKey=a;SecretKey=b;Bucket=c;Prefix=telemetry/prod;Debug=1;",
        );

        assert_eq!("telemetry/prod", parsed.prefix);
        assert!(parsed.debug);

        for value in ["0", "false", "no", "off"] {
            let parsed = S3ConnectionSettings::parse(&format!(
                "Endpoint=https://s3;Region=eu;AccessKey=a;SecretKey=b;Bucket=c;Prefix=p;Debug={}",
                value
            ));
            assert!(!parsed.debug, "'{}' should have left debug off", value);
        }
    }

    #[test]
    #[should_panic(expected = "'Bucket' is missing")]
    fn a_missing_key_is_loud() {
        S3ConnectionSettings::parse(
            "Endpoint=https://s3;Region=eu;AccessKey=a;SecretKey=b;Prefix=p",
        );
    }

    /// Two servers which shared the default folder would delete the hours of each other
    #[test]
    #[should_panic(expected = "'Prefix' is missing")]
    fn the_prefix_has_no_default() {
        S3ConnectionSettings::parse(
            "Endpoint=https://s3;Region=eu;AccessKey=a;SecretKey=b;Bucket=c",
        );
    }

    #[test]
    #[should_panic(expected = "'Prefix' is missing")]
    fn a_prefix_of_slashes_is_no_prefix() {
        S3ConnectionSettings::parse(
            "Endpoint=https://s3;Region=eu;AccessKey=a;SecretKey=b;Bucket=c;Prefix=/",
        );
    }

    #[test]
    #[should_panic(expected = "unknown key")]
    fn a_typo_is_loud() {
        S3ConnectionSettings::parse(
            "Endpoint=https://s3;Region=eu;AccessKey=a;SecretKey=b;Buckett=c;Prefix=p",
        );
    }

    #[test]
    #[should_panic(expected = "'Debug' expects 1/0")]
    fn an_unreadable_debug_value_is_loud() {
        S3ConnectionSettings::parse(
            "Endpoint=https://s3;Region=eu;AccessKey=a;SecretKey=b;Bucket=c;Prefix=p;Debug=maybe",
        );
    }
}
