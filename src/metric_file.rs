use rust_extensions::date_time::{HourKey, IntervalKey};

/// One hour on disk.
///
/// Normally a folder named by its hour key - `2026010512` - holding the data file and its
/// index. Legacy `metrics-<hourKey>.db` files from before the storage swap are described
/// the same way so that GC can still reach them.
#[derive(Debug)]
pub struct MetricFile {
    path: String,
    hour_key: IntervalKey<HourKey>,
    size: u64,
    is_folder: bool,
}

impl MetricFile {
    pub fn new(path: String, hour_key: IntervalKey<HourKey>, size: u64, is_folder: bool) -> Self {
        Self {
            path,
            hour_key,
            size,
            is_folder,
        }
    }

    pub fn get_hour_key(&self) -> IntervalKey<HourKey> {
        self.hour_key
    }

    /// Bytes the hour occupies - for a folder, everything inside it.
    pub fn get_file_size(&self) -> u64 {
        self.size
    }

    pub fn get_path_and_file_name(&self) -> &str {
        &self.path
    }

    pub fn is_folder(&self) -> bool {
        self.is_folder
    }
}

/// An hour folder is named by its key and nothing else, so the name is the key.
pub fn parse_hour_folder_name(name: &str) -> Option<IntervalKey<HourKey>> {
    if name.len() != 10 {
        return None;
    }

    let result = name.parse::<i64>().ok()?;

    Some(result.into())
}

/// `metrics-2026010512.db` and its `-wal` sibling, from before the storage swap.
pub fn parse_legacy_file_name(name: &str, prefix: &str) -> Option<IntervalKey<HourKey>> {
    let rest = name.strip_prefix(prefix)?.strip_prefix('-')?;

    if rest.len() < 10 {
        return None;
    }

    let result = rest[..10].parse::<i64>().ok()?;

    Some(result.into())
}
