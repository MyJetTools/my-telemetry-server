use std::collections::BTreeMap;

use rust_extensions::date_time::DateTimeAsMicroseconds;

use crate::{
    app_ctx::AppContext,
    metrics_storage::{hour_key_of, hour_start_of, HOUR_MICROS},
    reader_grpc::*,
};

/// The hours the product scope has metrics of - on the local disk and in S3 - the current
/// one always among them: it is where live metrics land, so it is worth looking at before
/// anything of it is written.
pub async fn get_available_hours_ago(app: &AppContext, scope: &str) -> Vec<AvailableFileGrpcModel> {
    let hours = app.repo.hours(scope).await;

    let current = hour_start_of(DateTimeAsMicroseconds::now().unix_microseconds);

    // hours ago -> (hour key, size)
    let mut result = BTreeMap::new();
    result.insert(0, (hour_key_of(current), 0));

    for (hour_start, size) in hours {
        let hours_ago = (current - hour_start) / HOUR_MICROS;

        // An hour ahead of the clock - a writer with a wrong one - has no "ago"
        if hours_ago >= 0 {
            result.insert(hours_ago, (hour_key_of(hour_start), size));
        }
    }

    result
        .into_iter()
        .map(|(hour_ago, (hour_key, file_size))| AvailableFileGrpcModel {
            hour_ago,
            file_size,
            hour_key,
        })
        .collect()
}
