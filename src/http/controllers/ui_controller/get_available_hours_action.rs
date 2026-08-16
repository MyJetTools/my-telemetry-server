use std::sync::Arc;

use my_http_server::{HttpContext, HttpFailResult, HttpOkResult, HttpOutput};
use rust_extensions::date_time::{DateTimeAsMicroseconds, HourKey, IntervalKey};

use crate::app_ctx::AppContext;

use super::models::*;

#[my_http_server::macros::http_route(
    method: "GET",
    route: "/ui/GetAvailableHours",
    controller: "ui",
    description: "Hours which have metrics on the disk",
    summary: "Hours which have metrics on the disk",
    result:[
        {status_code: 200, description: "Available hours", model="GetAvailableHoursResponse"},
    ]
)]
pub struct GetAvailableHoursAction {
    app: Arc<AppContext>,
}

impl GetAvailableHoursAction {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

async fn handle_request(
    action: &GetAvailableHoursAction,
    _ctx: &HttpContext,
) -> Result<HttpOkResult, HttpFailResult> {
    let available = crate::flows::get_available_hours_ago(&action.app).await;

    let mut hours: Vec<AvailableHourHttpModel> = available
        .into_iter()
        .map(|itm| AvailableHourHttpModel {
            hours_ago: itm.hour_ago,
            hour_key: calc_hour_key(itm.hour_ago),
            file_size: itm.file_size,
        })
        .collect();

    // The list is built from the metrics files on disk, so a server which has not
    // flushed anything yet reports nothing at all. The current hour is always a
    // legitimate thing to look at - it is where live events land - so it is added
    // here rather than left to the UI, which has no clock to derive its key from.
    if !hours.iter().any(|itm| itm.hours_ago == 0) {
        hours.insert(
            0,
            AvailableHourHttpModel {
                hours_ago: 0,
                hour_key: calc_hour_key(0),
                file_size: 0,
            },
        );
    }

    HttpOutput::as_json(GetAvailableHoursResponse { hours })
        .into_ok_result(true)
        .into()
}

fn calc_hour_key(hours_ago: i64) -> i64 {
    let mut dt = DateTimeAsMicroseconds::now();

    if hours_ago != 0 {
        dt.add_hours(-hours_ago.abs());
    }

    let interval: IntervalKey<HourKey> = dt.into();

    interval.to_i64()
}
