mod request_error;
pub use request_error::*;
mod time;
pub use time::*;
mod wire_ext;
pub use wire_ext::*;

// The wire models themselves come from the server's own crate, so the shapes this
// UI deserializes are the shapes the server serializes - by construction, not by
// convention. Anything client-only (view state, formatting helpers) lives here.
pub use rest_api_shared::{
    AvailableHourHttpModel, MetricByProcessModel, MetricHttpModel, ServiceHttpModel,
    ServiceOverviewContract, TechMetricsHttpModel,
};
