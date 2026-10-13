use flurl::{EmptyRequestModel, FlUrl, FlUrlError, FlUrlResponse, HttpVerb};
use my_http_utils::my_json::json_reader::{JsonValue, JsonValueReader, JsonValueRef};
use my_telemetry_ui_shared::*;

use crate::models::RequestError;

// Every URL here is relative: the wasm backend of FlUrl resolves it against the
// page origin, and the page is served by the very server these calls go to -
// my-telemetry-ui. Which my-telemetry-server it reads is the `env` of the request, and
// which product scope of it - the `scope`. A trace by process id has none: a process goes
// through the services of more than one product.

/// No limit on a 2xx body: it comes from our own host, and an hour of metrics
/// (`GetByServiceData`, `GetByProcessId`) is legitimately many megabytes.
const MAX_RESPONSE_BODY: usize = usize::MAX;

/// An error body is a message to show, not data.
const MAX_ERROR_BODY: usize = 64 * 1024;

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

async fn read_error_body(response: &mut FlUrlResponse) -> RequestError {
    let body = match response.get_body() {
        Ok(body) => body.into_string(MAX_ERROR_BODY).await,
        Err(err) => Err(err),
    };

    let message = body.unwrap_or_else(|err| format!("{:?}", err));

    RequestError { message }
}

/// 2xx -> read the body into `T` with its my-json reader; any other status -> `Err`
/// carrying the body.
async fn handle_http_response<T: for<'s> JsonValueReader<'s>>(
    response: Result<FlUrlResponse, FlUrlError>,
) -> Result<T, RequestError> {
    let mut response = response?;

    if is_success(response.get_status_code()) {
        let body = response.get_body()?.into_vec(MAX_RESPONSE_BODY).await?;
        let value = JsonValueRef::new(JsonValue::new(0, body.len()), &body);
        return Ok(T::from_json_value(&value)?);
    }

    Err(read_error_body(&mut response).await)
}

pub async fn get_app_info() -> Result<AppInfoHttpModel, RequestError> {
    let response = FlUrl::new("/ui/GetAppInfo")
        .execute_request(HttpVerb::Get, EmptyRequestModel)
        .await;

    handle_http_response(response).await
}

pub async fn get_product_scopes(env: &str) -> Result<ProductScopesHttpModel, RequestError> {
    let request = EnvHttpRequest {
        env: env.to_string(),
    };

    let response = FlUrl::new("/ui/GetProductScopes")
        .execute_request(HttpVerb::Get, request)
        .await;

    handle_http_response(response).await
}

pub async fn get_available_hours(
    env: &str,
    scope: &str,
) -> Result<Vec<AvailableHourHttpModel>, RequestError> {
    let request = GetAvailableHoursHttpInput {
        env: env.to_string(),
        scope: Some(scope.to_string()),
    };

    let response = FlUrl::new("/ui/GetAvailableHours")
        .execute_request(HttpVerb::Get, request)
        .await;

    let response: GetAvailableHoursResponse = handle_http_response(response).await?;
    Ok(response.hours)
}

pub async fn get_services(
    env: &str,
    scope: &str,
    hour_key: i64,
) -> Result<Vec<ServiceHttpModel>, RequestError> {
    let request = GetServicesHttpInput {
        env: env.to_string(),
        scope: Some(scope.to_string()),
        hour_key,
    };

    let response = FlUrl::new("/ui/GetServices")
        .execute_request(HttpVerb::Get, request)
        .await;

    let response: GetServicesResponse = handle_http_response(response).await?;
    Ok(response.services)
}

pub async fn get_service_overview(
    env: &str,
    scope: &str,
    hour_key: i64,
    service_id: String,
) -> Result<Vec<ServiceOverviewContract>, RequestError> {
    let request = GetServiceMetricsOverview {
        env: env.to_string(),
        scope: Some(scope.to_string()),
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
    env: &str,
    scope: &str,
    hour_key: i64,
    service_id: String,
    service_data: String,
    client_id: String,
    from_second_within_hour: i64,
) -> Result<Vec<MetricHttpModel>, RequestError> {
    let request = GetByServiceDataRequest {
        env: env.to_string(),
        scope: Some(scope.to_string()),
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
    env: &str,
    hour_key: i64,
    process_id: i64,
) -> Result<Vec<MetricByProcessModel>, RequestError> {
    let request = GetByProcessIdRequest {
        env: env.to_string(),
        process_id,
        hour_key,
    };

    let response = FlUrl::new("/ui/GetByProcessId")
        .execute_request(HttpVerb::Get, request)
        .await;

    let response: MetricsByProcessResponse = handle_http_response(response).await?;
    Ok(response.metrics)
}

pub async fn get_tech_metrics(env: &str) -> Result<TechMetricsHttpModel, RequestError> {
    let request = EnvHttpRequest {
        env: env.to_string(),
    };

    let response = FlUrl::new("/ui/GetTechMetrics")
        .execute_request(HttpVerb::Get, request)
        .await;

    handle_http_response(response).await
}
