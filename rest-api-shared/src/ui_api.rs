use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

/// A tag attached to a metric event. The UI renders these as key/value pairs, so
/// they cross the wire structured rather than as a debug-formatted string.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TagHttpModel {
    pub key: String,
    pub value: String,
}

// ---- /ui/GetAvailableHours -------------------------------------------------

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GetAvailableHoursResponse {
    pub hours: Vec<AvailableHourHttpModel>,
}

/// `hour_key` is resolved by the server on purpose: the UI runs in the browser as
/// wasm, where there is no `SystemTime`, so it must never compute an hour key of
/// its own. It picks an entry here and echoes the key back on every other call.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct AvailableHourHttpModel {
    pub hours_ago: i64,
    pub hour_key: i64,
    pub file_size: u64,
}

// ---- /ui/GetServices -------------------------------------------------------

#[derive(Debug, MyHttpInput)]
pub struct GetServicesHttpInput {
    #[http_query(description = "Hour key")]
    pub hour_key: i64,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GetServicesResponse {
    pub services: Vec<ServiceHttpModel>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ServiceHttpModel {
    pub id: String,
    pub avg: i64,
    pub amount: i64,
}

// ---- /ui/GetServiceOverview ------------------------------------------------

#[derive(Debug, MyHttpInput)]
pub struct GetServiceMetricsOverview {
    #[http_query(description = "Id of service")]
    pub id: String,

    #[http_query(description = "Hour key")]
    pub hour_key: i64,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GetServiceOverviewResponse {
    pub data: Vec<ServiceOverviewContract>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
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

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct MetricsResponse {
    pub metrics: Vec<MetricHttpModel>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct MetricHttpModel {
    pub id: i64,
    pub started: i64,
    pub duration: i64,
    pub success: Option<String>,
    pub error: Option<String>,
    pub tags: Vec<TagHttpModel>,
}

// ---- /ui/GetByProcessId ----------------------------------------------------

#[derive(Debug, MyHttpInput)]
pub struct GetByProcessIdRequest {
    #[http_query(name: "processId"; description = "Id of service")]
    pub process_id: i64,
    #[http_query(name: "hour_key"; description = "Hour key")]
    pub hour_key: i64,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct MetricsByProcessResponse {
    pub metrics: Vec<MetricByProcessModel>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
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

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
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
