use std::{collections::BTreeMap, sync::Arc};

use rust_extensions::{
    date_time::{HourKey, IntervalKey},
    InsertOrUpdateEntry2Keys, SortedVecWith2StrKey,
};

use crate::db::*;

use super::{AppDataHourStatistics, HourScope};

/// By hour and product scope: the statistics of each action of each service
pub struct StatisticsByAppAndData {
    data: BTreeMap<HourScope, SortedVecWith2StrKey<AppDataHourStatistics>>,
    to_persist: BTreeMap<HourScope, SortedVecWith2StrKey<AppDataHourStatistics>>,
}

impl StatisticsByAppAndData {
    pub fn new() -> Self {
        Self {
            data: BTreeMap::new(),
            to_persist: BTreeMap::new(),
        }
    }

    /// `events` are the ones of the hour which went to the scope
    pub fn update(
        &mut self,
        hour_key: IntervalKey<HourKey>,
        scope: &Arc<str>,
        events: &[MetricDto],
    ) {
        let key: HourScope = (hour_key.to_i64(), scope.clone());

        let sub_items = self.data.entry(key.clone()).or_default();

        let mut to_persist = Vec::new();

        for itm in events {
            match sub_items.insert_or_update(&itm.name, &itm.data) {
                InsertOrUpdateEntry2Keys::Insert(entry) => {
                    let itm = AppDataHourStatistics::new(itm);
                    entry.insert(itm.clone());
                    to_persist.push(itm);
                }
                InsertOrUpdateEntry2Keys::Update(mut entry) => {
                    let itm_to_persist = entry.get_item_mut().update(itm);
                    to_persist.push(itm_to_persist);
                }
            }
        }

        self.set_to_persist(key, to_persist);
    }

    fn set_to_persist(&mut self, key: HourScope, events_to_persist: Vec<AppDataHourStatistics>) {
        let sub_items = self.to_persist.entry(key).or_default();

        for itm in events_to_persist {
            match sub_items.insert_or_update(&itm.service, &itm.data) {
                InsertOrUpdateEntry2Keys::Insert(entry) => {
                    entry.insert(itm);
                }
                InsertOrUpdateEntry2Keys::Update(mut entry) => {
                    entry.get_item_mut().update_to_persist(&itm);
                }
            }
        }
    }

    /// The statistics of the hour as the database has them, of every scope
    pub fn restore(
        &mut self,
        hour_key: IntervalKey<HourKey>,
        metric_by_hour: Vec<HourAppDataStatisticsDto>,
    ) {
        for itm in metric_by_hour {
            let key: HourScope = (hour_key.to_i64(), itm.scope.as_str().into());
            let itm: AppDataHourStatistics = itm.into();

            self.data.entry(key).or_default().insert_or_replace(itm);
        }
    }

    pub fn get_to_persist(
        &mut self,
    ) -> Option<BTreeMap<HourScope, SortedVecWith2StrKey<AppDataHourStatistics>>> {
        if self.to_persist.is_empty() {
            return None;
        }
        let mut result = BTreeMap::new();

        std::mem::swap(&mut result, &mut self.to_persist);

        Some(result)
    }

    pub fn get<TResult>(
        &self,
        hour_key: IntervalKey<HourKey>,
        scope: &str,
        app: &str,
        convert: impl Fn(&AppDataHourStatistics) -> TResult,
    ) -> Option<Vec<TResult>> {
        let items = self.data.get(&(hour_key.to_i64(), Arc::from(scope)))?;

        let sub_items = items.get_by_primary_key(app)?;

        let mut result = Vec::with_capacity(self.data.len());

        for itm in sub_items {
            result.push(convert(itm));
        }

        Some(result)
    }

    pub fn gc_old_data(&mut self, from_hour: IntervalKey<HourKey>) {
        let to_gc = {
            let mut to_gc = Vec::new();
            for key in self.data.keys() {
                if key.0 <= from_hour.to_i64() {
                    to_gc.push(key.clone());
                }
            }

            to_gc
        };

        //if !to_gc.is_empty() {
        //println!("GC hour_service_name {:?}", to_gc);
        // }

        for key in to_gc {
            self.data.remove(&key);
        }
    }

    pub fn get_queue_hours_size(&self) -> (usize, usize) {
        (self.data.len(), self.to_persist.len())
    }

    pub fn get_size_and_capacity(&self) -> (usize, usize) {
        let mut size = 0;
        let mut capacity = 0;

        for (_, items) in self.data.iter() {
            let (l, c) = items.get_len_and_capacity();
            size += l;
            capacity += c;
        }

        (size, capacity)
    }
}
