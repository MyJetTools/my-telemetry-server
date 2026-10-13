use std::{collections::BTreeMap, sync::Arc};

use rust_extensions::{
    date_time::{HourKey, IntervalKey},
    sorted_vec::{EntityWithStrKey, InsertOrUpdateEntry, SortedVecWithStrKey},
};

use crate::db::*;

use super::HourScope;

#[derive(Debug, Clone)]
pub struct AppDurationStatistics {
    pub name: String,
    pub amount: i64,
    pub duration_micros: i64,
}

impl EntityWithStrKey for AppDurationStatistics {
    fn get_key(&self) -> &str {
        self.name.as_str()
    }
}

impl AppDurationStatistics {
    pub fn new(name: String, duration_micros: i64) -> Self {
        Self {
            name,
            amount: 1,
            duration_micros,
        }
    }

    pub fn update(&mut self, other: Self) {
        *self = other;
    }
    pub fn inc(&mut self, duration_micros: i64) -> AppDurationStatistics {
        self.amount += 1;
        self.duration_micros += duration_micros;

        self.clone()
    }
}

impl Into<AppDurationStatistics> for HourStatisticsDto {
    fn into(self) -> AppDurationStatistics {
        AppDurationStatistics {
            name: self.app,
            amount: self.amount,
            duration_micros: self.duration_micros,
        }
    }
}

/// By hour and product scope: how many events each service has and how long they took
pub struct EventAmountsByHour {
    pub items: BTreeMap<HourScope, SortedVecWithStrKey<AppDurationStatistics>>,
    pub to_persist: BTreeMap<HourScope, SortedVecWithStrKey<AppDurationStatistics>>,
}

impl EventAmountsByHour {
    pub fn new() -> Self {
        Self {
            items: BTreeMap::new(),
            to_persist: BTreeMap::new(),
        }
    }

    pub fn inc(
        &mut self,
        interval_key: IntervalKey<HourKey>,
        scope: &Arc<str>,
        metric_dto: &MetricDto,
    ) {
        let key: HourScope = (interval_key.to_i64(), scope.clone());

        let items = self.items.entry(key.clone()).or_default();

        let to_persist = match items.insert_or_update(metric_dto.name.as_str()) {
            InsertOrUpdateEntry::Insert(entry) => {
                let item = AppDurationStatistics::new(
                    metric_dto.name.to_string(),
                    metric_dto.duration_micro,
                );
                entry.insert(item.clone());
                item
            }
            InsertOrUpdateEntry::Update(update_entry) => {
                update_entry.item.inc(metric_dto.duration_micro);
                update_entry.item.clone()
            }
        };

        self.engage_persist(key, metric_dto.name.as_str(), to_persist);
    }

    fn engage_persist(&mut self, key: HourScope, app: &str, to_persist: AppDurationStatistics) {
        let sub_items = self.to_persist.entry(key).or_default();

        match sub_items.get_mut(app) {
            Some(count) => {
                count.update(to_persist);
            }
            None => {
                sub_items.insert_or_replace(to_persist);
            }
        }
    }

    pub fn get_to_persist(
        &mut self,
    ) -> Option<BTreeMap<HourScope, SortedVecWithStrKey<AppDurationStatistics>>> {
        if self.to_persist.is_empty() {
            return None;
        }

        let mut result = BTreeMap::new();

        std::mem::swap(&mut self.to_persist, &mut result);

        Some(result)
    }

    /// The statistics of the hour as the database has them, of every scope
    pub fn restore(
        &mut self,
        hour_key: IntervalKey<HourKey>,
        items_to_restore: impl Iterator<Item = HourStatisticsDto>,
    ) {
        for itm in items_to_restore {
            let key: HourScope = (hour_key.to_i64(), itm.scope.as_str().into());

            self.items
                .entry(key)
                .or_default()
                .insert_or_replace(itm.into());
        }
    }

    pub fn get<TResult>(
        &self,
        hour_key: IntervalKey<HourKey>,
        scope: &str,
        convert: impl Fn(&AppDurationStatistics) -> TResult,
    ) -> Option<Vec<TResult>> {
        let items = self.items.get(&(hour_key.to_i64(), Arc::from(scope)))?;

        let mut result = Vec::with_capacity(items.len());

        for itm in items.iter() {
            result.push(convert(itm));
        }

        Some(result)
    }

    pub fn gc_old_data(&mut self, from_hour: IntervalKey<HourKey>) {
        let to_gc = {
            let mut to_gc = Vec::new();
            for key in self.items.keys() {
                if key.0 <= from_hour.to_i64() {
                    to_gc.push(key.clone());
                }
            }

            to_gc
        };

        if !to_gc.is_empty() {
            println!("GC amounts_by_hour {:?}", to_gc);
        }

        for key in to_gc {
            self.items.remove(&key);
        }
    }
}
