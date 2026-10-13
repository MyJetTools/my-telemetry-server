use std::collections::BTreeMap;

use rust_extensions::SortedVecWith2StrKey;

use crate::{
    app_ctx::AppContext,
    caches::{AppDataHourStatistics, HourScope},
    db::HourAppDataStatisticsDto,
};

pub async fn write_hour_app_data_statistics(
    app: &AppContext,
    metrics_to_save: BTreeMap<HourScope, SortedVecWith2StrKey<AppDataHourStatistics>>,
) {
    let mut dto_to_insert = Vec::new();

    for ((hour_key, scope), items) in metrics_to_save.iter() {
        for item in items.iter() {
            dto_to_insert.push(HourAppDataStatisticsDto {
                scope: scope.to_string(),
                service: item.service.to_string(),
                data_hashed: crate::db::data_hashed::calc(&item.data),
                hour_key: *hour_key,
                data: item.data.to_string(),
                max: item.max,
                min: item.min,
                errors_amount: item.errors_amount,
                success_amount: item.success_amount,
                sum_of_duration: item.sum_of_duration,
                amount: item.amount,
            })
        }
    }

    app.hour_app_data_statistics_repo
        .update_metrics(&dto_to_insert)
        .await;
}
