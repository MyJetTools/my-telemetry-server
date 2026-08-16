use flurl::{EmptyRequestModel, FlUrl, FlUrlError, FlUrlResponse, HttpVerb};
use rest_api_shared::*;
use serde::de::DeserializeOwned;

use crate::models::RequestError;

// Every URL here is relative: the wasm backend of FlUrl resolves it against the
// page origin, and the page is served by the very server these calls go to. There
// is deliberately no base URL and no environment selector - unlike the standalone
// my-telemetry-ui, this UI only ever talks to its own host.

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

async fn read_error_body(response: &mut FlUrlResponse) -> RequestError {
    let message = response
        .get_body_as_str()
        .await
        .map(|body| body.to_string())
        .unwrap_or_else(|err| format!("{:?}", err));

    RequestError { message }
}

/// 2xx -> deserialize the body into `T`; any other status -> `Err` carrying the body.
async fn handle_http_response<T: DeserializeOwned>(
    response: Result<FlUrlResponse, FlUrlError>,
) -> Result<T, RequestError> {
    let mut response = response?;

    if is_success(response.get_status_code()) {
        return Ok(response.get_json().await?);
    }

    Err(read_error_body(&mut response).await)
}

pub async fn get_available_hours() -> Result<Vec<AvailableHourHttpModel>, RequestError> {
    let response = FlUrl::new("/ui/GetAvailableHours")
        .execute_request(HttpVerb::Get, EmptyRequestModel)
        .await;

    let response: GetAvailableHoursResponse = handle_http_response(response).await?;
    Ok(response.hours)
}

pub async fn get_services(hour_key: i64) -> Result<Vec<ServiceHttpModel>, RequestError> {
    let response = FlUrl::new("/ui/GetServices")
        .execute_request(HttpVerb::Get, GetServicesHttpInput { hour_key })
        .await;

    let response: GetServicesResponse = handle_http_response(response).await?;
    Ok(response.services)
}

pub async fn get_service_overview(
    hour_key: i64,
    service_id: String,
) -> Result<Vec<ServiceOverviewContract>, RequestError> {
    let request = GetServiceMetricsOverview {
        id: service_id,
        hour_key,
    };

    let response = FlUrl::new("/ui/GetServiceOverview")
        .execute_request(HttpVerb::Get, request)
        .await;

    let response: GetServiceOverviewResponse = handle_http_response(response).await?;
    Ok(response.data)
}

pub async fn get_by_service_data(
    hour_key: i64,
    service_id: String,
    service_data: String,
    client_id: String,
    from_second_within_hour: i64,
) -> Result<Vec<MetricHttpModel>, RequestError> {
    let request = GetByServiceDataRequest {
        id: service_id,
        data: service_data,
        hour_key,
        // An empty box means "no filter". Sending it as an absent parameter keeps
        // the server from matching on an empty client id.
        client_id: if client_id.is_empty() {
            None
        } else {
            Some(client_id)
        },
        from_second_within_hour,
    };

    let response = FlUrl::new("/ui/GetByServiceData")
        .execute_request(HttpVerb::Get, request)
        .await;

    let response: MetricsResponse = handle_http_response(response).await?;
    Ok(response.metrics)
}

pub async fn get_by_process_id(
    hour_key: i64,
    process_id: i64,
) -> Result<Vec<MetricByProcessModel>, RequestError> {
    let request = GetByProcessIdRequest {
        process_id,
        hour_key,
    };

    let response = FlUrl::new("/ui/GetByProcessId")
        .execute_request(HttpVerb::Get, request)
        .await;

    let response: MetricsByProcessResponse = handle_http_response(response).await?;
    Ok(response.metrics)
}

pub async fn get_tech_metrics() -> Result<TechMetricsHttpModel, RequestError> {
    let response = FlUrl::new("/ui/GetTechMetrics")
        .execute_request(HttpVerb::Get, EmptyRequestModel)
        .await;

    handle_http_response(response).await
}
