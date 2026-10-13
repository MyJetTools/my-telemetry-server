use rust_extensions::EntityWith2StrKey;

use crate::db::*;

#[derive(Clone)]
pub struct AppDataHourStatistics {
    pub service: String,
    pub data: String,
    pub min: i64,
    pub max: i64,
    pub errors_amount: i64,
    pub success_amount: i64,
    pub sum_of_duration: i64,
    pub amount: i64,
}

impl Into<AppDataHourStatistics> for HourAppDataStatisticsDto {
    fn into(self) -> AppDataHourStatistics {
        AppDataHourStatistics {
            service: self.service,
            data: self.data,
            min: self.min,
            max: self.max,
            errors_amount: self.errors_amount,
            success_amount: self.success_amount,
            sum_of_duration: self.sum_of_duration,
            amount: self.amount,
        }
    }
}

impl EntityWith2StrKey for AppDataHourStatistics {
    fn get_primary_key(&self) -> &str {
        self.service.as_str()
    }

    fn get_secondary_key(&self) -> &str {
        self.data.as_str()
    }
}

impl AppDataHourStatistics {
    pub fn new(itm: &MetricDto) -> Self {
        Self {
            service: itm.name.to_string(),
            data: itm.data.to_string(),
            min: itm.duration_micro,
            max: itm.duration_micro,
            errors_amount: if itm.fail.is_some() { 1 } else { 0 },
            success_amount: if itm.success.is_some() { 1 } else { 0 },
            sum_of_duration: itm.duration_micro,
            amount: 1,
        }
    }
    pub fn update(&mut self, itm: &MetricDto) -> Self {
        if itm.duration_micro < self.min || self.min == 0 {
            self.min = itm.duration_micro;
        }

        if itm.duration_micro > self.max {
            self.max = itm.duration_micro;
        }

        self.sum_of_duration += itm.duration_micro;
        self.amount += 1;

        if itm.success.is_some() {
            self.success_amount += 1;
        }

        if itm.fail.is_some() {
            self.errors_amount += 1;
        }

        self.clone()
    }

    pub fn update_to_persist(&mut self, itm: &AppDataHourStatistics) {
        self.max = itm.max;
        self.min = itm.min;
        self.errors_amount = itm.errors_amount;
        self.success_amount = itm.success_amount;
        self.sum_of_duration = itm.sum_of_duration;
        self.amount = itm.amount;
    }
}
