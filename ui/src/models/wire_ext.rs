use std::time::Duration;

use rust_extensions::date_time::DateTimeAsMicroseconds;

use rest_api_shared::{
    MetricByProcessModel, MetricHttpModel, ServiceHttpModel, ServiceOverviewContract,
};

// The shared wire models stay pure data - every presentational helper the views
// need is bolted on here, on the consumer side.

pub trait ServiceHttpModelExt {
    fn get_avg_duration(&self) -> Duration;
}

impl ServiceHttpModelExt for ServiceHttpModel {
    fn get_avg_duration(&self) -> Duration {
        Duration::from_micros(self.avg as u64)
    }
}

pub trait ServiceOverviewContractExt {
    fn get_min_duration(&self) -> Duration;
    fn get_max_duration(&self) -> Duration;
    fn get_avg_duration(&self) -> Duration;
}

impl ServiceOverviewContractExt for ServiceOverviewContract {
    fn get_min_duration(&self) -> Duration {
        Duration::from_micros(self.min as u64)
    }

    fn get_max_duration(&self) -> Duration {
        Duration::from_micros(self.max as u64)
    }

    fn get_avg_duration(&self) -> Duration {
        Duration::from_micros(self.avg as u64)
    }
}

pub trait MetricHttpModelExt {
    fn get_started(&self) -> DateTimeAsMicroseconds;
    fn get_duration(&self) -> Duration;
}

impl MetricHttpModelExt for MetricHttpModel {
    fn get_started(&self) -> DateTimeAsMicroseconds {
        DateTimeAsMicroseconds::new(self.started)
    }

    fn get_duration(&self) -> Duration {
        Duration::from_micros(self.duration as u64)
    }
}

pub trait MetricByProcessModelExt {
    fn get_started(&self) -> DateTimeAsMicroseconds;
    fn get_duration(&self) -> Duration;
}

impl MetricByProcessModelExt for MetricByProcessModel {
    fn get_started(&self) -> DateTimeAsMicroseconds {
        DateTimeAsMicroseconds::new(self.started)
    }

    fn get_duration(&self) -> Duration {
        Duration::from_micros(self.duration as u64)
    }
}
