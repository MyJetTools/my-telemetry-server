use std::collections::BTreeMap;

use rust_extensions::sorted_vec::SortedVecWithStrKey;

use crate::{
    app_ctx::AppContext,
    caches::{AppDurationStatistics, HourScope},
    db::HourStatisticsDto,
};

pub async fn write_hour_statistics_to_db(
    app: &AppContext,
    metrics_to_save: BTreeMap<HourScope, SortedVecWithStrKey<AppDurationStatistics>>,
) {
    let mut dto = Vec::new();

    for ((hour_key, scope), items) in metrics_to_save {
        for item in items.into_vec() {
            dto.push(HourStatisticsDto {
                hour_key,
                scope: scope.to_string(),
                app: item.name,
                duration_micros: item.duration_micros,
                amount: item.amount,
            });
        }
    }

    app.hour_statistics_repo.update(dto.as_slice()).await;
}
