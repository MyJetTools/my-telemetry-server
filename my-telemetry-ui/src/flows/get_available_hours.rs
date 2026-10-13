use my_telemetry_ui_shared::AvailableHourHttpModel;
use rust_extensions::date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey};

use crate::{
    app::AppContext,
    reader_grpc::{AvailableFileGrpcModel, GetAvailableHoursRequest},
};

use super::FlowError;

/// The hours the product scope of the env has, newest first, the current one always
/// among them.
pub async fn get_available_hours(
    app: &AppContext,
    env: &str,
    scope: Option<String>,
) -> Result<Vec<AvailableHourHttpModel>, FlowError> {
    let grpc_client = crate::scripts::get_grpc_client(app, env).await?;

    let hours: Vec<AvailableFileGrpcModel> = grpc_client
        .get_available_hours_ago(GetAvailableHoursRequest {
            product_scope: scope,
        })
        .await?
        .into_vec()
        .await?;

    let mut result: Vec<AvailableHourHttpModel> = hours
        .into_iter()
        .map(|hour| AvailableHourHttpModel {
            hours_ago: hour.hour_ago,
            // A server older than `HourKey` sends 0: then it is derived the way that server
            // derived it, from how many hours ago it was
            hour_key: if hour.hour_key == 0 {
                hour_key_of(hour.hour_ago)
            } else {
                hour.hour_key
            },
            file_size: hour.file_size,
        })
        .collect();

    // A server which sends `HourKey` sends the current hour too; an older one only when
    // it has flushed something into it
    if !result.iter().any(|hour| hour.hours_ago == 0) {
        result.push(AvailableHourHttpModel {
            hours_ago: 0,
            hour_key: hour_key_of(0),
            file_size: 0,
        });
    }

    result.sort_by_key(|hour| hour.hours_ago);

    Ok(result)
}

fn hour_key_of(hours_ago: i64) -> i64 {
    let mut now = DateTimeAsMicroseconds::now();

    if hours_ago != 0 {
        now.add_hours(-hours_ago.abs());
    }

    let hour_key: IntervalKey<HourKey> = now.into();
    hour_key.to_i64()
}
