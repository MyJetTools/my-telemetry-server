use crate::{
    app_ctx::{AppContext, METRICS_FILE_PREFIX},
    metric_file::*,
};

/// Every hour present on disk.
///
/// An hour is a folder named by its key - `2026010512` - so the scan is one level deep and
/// the size of an hour is the sum of what the folder holds. Loose `metrics-*.db` files left
/// over from before the storage swap are picked up too, so GC eventually clears them.
pub async fn get_metrics_files(app: &AppContext) -> Vec<MetricFile> {
    let path_to_scan = app.settings_reader.get_db_path().await;

    let mut dir_entry = match tokio::fs::read_dir(path_to_scan).await {
        Ok(result) => result,
        Err(err) => {
            println!("Failed to scan metrics folder: {:?}", err);
            return vec![];
        }
    };

    let mut result = Vec::new();

    while let Ok(Some(entry)) = dir_entry.next_entry().await {
        let path = entry.path();

        let Some(name) = path.file_name().and_then(|itm| itm.to_str()) else {
            continue;
        };

        let Some(full_path) = path.as_os_str().to_str() else {
            continue;
        };

        if path.is_dir() {
            let Some(hour_key) = parse_hour_folder_name(name) else {
                continue;
            };

            result.push(MetricFile::new(
                full_path.to_string(),
                hour_key,
                folder_size(&path).await,
                true,
            ));

            continue;
        }

        let Some(hour_key) = parse_legacy_file_name(name, METRICS_FILE_PREFIX) else {
            continue;
        };

        let size = entry.metadata().await.map(|itm| itm.len()).unwrap_or(0);

        result.push(MetricFile::new(
            full_path.to_string(),
            hour_key,
            size,
            false,
        ));
    }

    result
}

async fn folder_size(path: &std::path::Path) -> u64 {
    let Ok(mut dir_entry) = tokio::fs::read_dir(path).await else {
        return 0;
    };

    let mut result = 0;

    while let Ok(Some(entry)) = dir_entry.next_entry().await {
        if let Ok(metadata) = entry.metadata().await {
            if metadata.is_file() {
                result += metadata.len();
            }
        }
    }

    result
}
