use my_http_utils::macros::{MyHttpInput, MyHttpInputObjectStructure};
use serde::Serialize;

// Responses derive `MyHttpInputObjectStructure`, not `MyHttpObjectStructure`, although they are
// outputs: it is the one that also emits the my-json `JsonValueReader`, which is how the wasm UI
// reads them - no serde on the client. The schema it gives the server is the very same.
// `Serialize` stays only because `HttpOutput::as_json` still writes through serde.

/// A tag attached to a metric event. The UI renders these as key/value pairs, so
/// they cross the wire structured rather than as a debug-formatted string.
#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct TagHttpModel {
    pub key: String,
    pub value: String,
}

// ---- /ui/GetAppInfo --------------------------------------------------------

/// What the UI needs before its first request: the envs - the my-telemetry-servers it can
/// read, by their names in the settings - and the version of the UI server.
#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct AppInfoHttpModel {
    pub version: String,
    pub envs: Vec<String>,
}

/// The input of every request which needs nothing but the env.
#[derive(Debug, MyHttpInput)]
pub struct EnvHttpRequest {
    #[http_query(description = "The my-telemetry-server to read, by its name in the settings")]
    pub env: String,
}

// ---- /ui/GetProductScopes --------------------------------------------------

/// The product scopes the env has metrics of, the default one first. A writer names the
/// scope of its metrics - `product` of its connection string; the metrics of the writers
/// which name none are in the default one. The hours, the services and the events are
/// read by scope: a request which names none reads the default one.
#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct ProductScopesHttpModel {
    pub default_scope: String,
    pub scopes: Vec<String>,
}

// ---- /ui/GetAvailableHours -------------------------------------------------

#[derive(Debug, MyHttpInput)]
pub struct GetAvailableHoursHttpInput {
    #[http_query(description = "The my-telemetry-server to read, by its name in the settings")]
    pub env: String,
    #[http_query(description = "The product scope to read; the default one when it is not given")]
    pub scope: Option<String>,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct GetAvailableHoursResponse {
    pub hours: Vec<AvailableHourHttpModel>,
}

/// `hour_key` is resolved by the server on purpose: the UI runs in the browser as
/// wasm, where there is no `SystemTime`, so it must never compute an hour key of
/// its own. It picks an entry here and echoes the key back on every other call.
#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct AvailableHourHttpModel {
    pub hours_ago: i64,
    pub hour_key: i64,
    pub file_size: u64,
}

// ---- /ui/GetServices -------------------------------------------------------

#[derive(Debug, MyHttpInput)]
pub struct GetServicesHttpInput {
    #[http_query(description = "The my-telemetry-server to read, by its name in the settings")]
    pub env: String,
    #[http_query(description = "The product scope to read; the default one when it is not given")]
    pub scope: Option<String>,
    #[http_query(description = "Hour key")]
    pub hour_key: i64,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct GetServicesResponse {
    pub services: Vec<ServiceHttpModel>,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct ServiceHttpModel {
    pub id: String,
    pub avg: i64,
    pub amount: i64,
}

// ---- /ui/GetServiceOverview ------------------------------------------------

#[derive(Debug, MyHttpInput)]
pub struct GetServiceMetricsOverview {
    #[http_query(description = "The my-telemetry-server to read, by its name in the settings")]
    pub env: String,
    #[http_query(description = "The product scope to read; the default one when it is not given")]
    pub scope: Option<String>,
    #[http_query(description = "Id of service")]
    pub id: String,

    #[http_query(description = "Hour key")]
    pub hour_key: i64,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct GetServiceOverviewResponse {
    pub data: Vec<ServiceOverviewContract>,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct ServiceOverviewContract {
    pub data: String,
    pub min: i64,
    pub max: i64,
    pub avg: i64,
    pub success: i64,
    pub error: i64,
    pub total: i64,
}

// ---- /ui/GetByServiceData --------------------------------------------------

#[derive(Debug, MyHttpInput)]
pub struct GetByServiceDataRequest {
    #[http_query(description = "The my-telemetry-server to read, by its name in the settings")]
    pub env: String,
    #[http_query(description = "The product scope to read; the default one when it is not given")]
    pub scope: Option<String>,
    #[http_query(description = "Id of service")]
    pub id: String,
    #[http_query(description = "Data of the service")]
    pub data: String,
    #[http_query(name:"hourKey", description = "Hour Key")]
    pub hour_key: i64,
    #[http_query(name:"clientId", description = "Client Id")]
    pub client_id: Option<String>,

    #[http_query(name:"fromSecondWithinHour", description = "Second within hour")]
    pub from_second_within_hour: i64,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct MetricsResponse {
    pub metrics: Vec<MetricHttpModel>,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct MetricHttpModel {
    pub id: i64,
    pub started: i64,
    pub duration: i64,
    pub success: Option<String>,
    pub error: Option<String>,
    pub tags: Vec<TagHttpModel>,
}

// ---- /ui/GetByProcessId ----------------------------------------------------

/// No product scope here: a process goes through the services of more than one, so its
/// events are looked for in all of them.
#[derive(Debug, MyHttpInput)]
pub struct GetByProcessIdRequest {
    #[http_query(description = "The my-telemetry-server to read, by its name in the settings")]
    pub env: String,
    #[http_query(name: "processId"; description = "Id of service")]
    pub process_id: i64,
    #[http_query(name: "hour_key"; description = "Hour key")]
    pub hour_key: i64,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct MetricsByProcessResponse {
    pub metrics: Vec<MetricByProcessModel>,
}

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct MetricByProcessModel {
    pub id: String,
    pub data: String,
    pub started: i64,
    pub duration: i64,
    pub success: Option<String>,
    pub error: Option<String>,
    pub tags: Vec<TagHttpModel>,
}

// ---- /ui/GetTechMetrics ----------------------------------------------------

#[derive(Serialize, MyHttpInputObjectStructure, Clone, Debug, PartialEq)]
pub struct TechMetricsHttpModel {
    pub app_data_hours_size: u64,
    pub app_data_to_persist_hours_size: u64,
    pub queue_size: u64,
    pub queue_capacity: u64,
    pub queue_by_process_size: u64,
    pub queue_by_process_capacity: u64,
    pub user_id_links_size: u64,
    pub user_id_links_capacity: u64,
    pub app_data_size: u64,
    pub app_data_capacity: u64,
    /// Metrics accepted but still in the storage's in-memory window - what a crash costs.
    pub storage_window_size: u64,
}

#[cfg(test)]
mod tests {
    use my_http_utils::my_json::json_reader::{JsonValue, JsonValueReader, JsonValueRef};

    use super::*;

    /// The server writes a response with serde, the UI reads it with my-json: two
    /// implementations of one contract, so it is pinned here rather than in the browser.
    fn round_trip<T: Serialize + for<'s> JsonValueReader<'s>>(model: &T) -> T {
        let json = serde_json::to_vec(model).unwrap();
        let value = JsonValueRef::new(JsonValue::new(0, json.len()), &json);
        T::from_json_value(&value).unwrap()
    }

    #[test]
    fn metrics_response_survives_serde_to_my_json() {
        let model = MetricsResponse {
            metrics: vec![
                MetricHttpModel {
                    id: -1,
                    started: 1_767_614_400_000_000,
                    duration: 1234,
                    success: Some("ok \"quoted\" \\ Ф".to_string()),
                    error: None,
                    tags: vec![TagHttpModel {
                        key: "client_id".to_string(),
                        value: "abc".to_string(),
                    }],
                },
                MetricHttpModel {
                    id: 2,
                    started: 0,
                    duration: 0,
                    success: None,
                    error: Some("stack\ntrace".to_string()),
                    tags: vec![],
                },
            ],
        };

        assert_eq!(round_trip(&model), model);
    }

    #[test]
    fn product_scopes_survive_serde_to_my_json() {
        let model = ProductScopesHttpModel {
            default_scope: "default".to_string(),
            scopes: vec!["default".to_string(), "my-product_1".to_string()],
        };

        assert_eq!(round_trip(&model), model);
    }

    #[test]
    fn tech_metrics_survive_serde_to_my_json() {
        let model = TechMetricsHttpModel {
            app_data_hours_size: u64::MAX,
            app_data_to_persist_hours_size: 1,
            queue_size: 2,
            queue_capacity: 3,
            queue_by_process_size: 4,
            queue_by_process_capacity: 5,
            user_id_links_size: 6,
            user_id_links_capacity: 7,
            app_data_size: 8,
            app_data_capacity: 9,
            storage_window_size: 10,
        };

        assert_eq!(round_trip(&model), model);
    }
}
