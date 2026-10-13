use std::sync::Arc;

/// What the statistics are kept by: the hour and the product scope the metrics went to
pub type HourScope = (i64, Arc<str>);

mod statistics_by_hour_and_service_name;
pub use statistics_by_hour_and_service_name::*;
//mod metrics_by_action_name;
//pub use metrics_by_action_name::*;
mod itm;
pub use itm::*;
mod app_duration_statistics;
pub use app_duration_statistics::*;
