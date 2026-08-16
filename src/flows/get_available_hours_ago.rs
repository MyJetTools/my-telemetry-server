use std::collections::BTreeMap;

use rust_extensions::date_time::DateTimeAsMicroseconds;

use crate::{app_ctx::AppContext, reader_grpc::*};

pub async fn get_available_hours_ago(app: &AppContext) -> Vec<AvailableFileGrpcModel> {
    let metric_files = crate::scripts::get_metrics_files(app).await;

    let now = DateTimeAsMicroseconds::now();

    let mut result = BTreeMap::new();
    for metric_file in metric_files {
        if let Ok(hour) = metric_file.get_hour_key().try_to_date_time() {
            let diff = now - hour;

            let hours = diff.get_full_hours();

            // Summed rather than assigned: an hour is normally one folder, but a leftover
            // `.db` from before the storage swap can carry the same hour key, and
            // overwriting would report whichever the scan yielded last.
            *result.entry(hours).or_insert(0) += metric_file.get_file_size();
        }
    }

    /*
       let path_to_scan = app.settings_reader.get_db_path().await;

       let mut dir_entry = tokio::fs::read_dir(path_to_scan).await.unwrap();

       while let Some(entry) = dir_entry.next_entry().await.unwrap() {
           if !entry.path().is_file() {
               continue;
           }

           let path = entry.path();

           if let Some(file_name) = path.file_name() {
               if let Some(file_name) = file_name.to_str() {
                   if entry.path().starts_with(METRICS_FILE_PREFIX) {
                       continue;
                   }

                   let hour_key = get_hour_key(file_name);

                   if let Some(hour_key) = hour_key {
                       if let Ok(hour) = hour_key.try_to_date_time() {
                           let diff = now - hour;

                           let hours = diff.get_full_hours();

                           let file_metadata = entry.metadata().await.unwrap();
                           result.insert(hours, file_metadata.len());
                       }
                   }
               }
           }
       }
    */
    result
        .into_iter()
        .map(|(hour_ago, file_size)| AvailableFileGrpcModel {
            hour_ago,
            file_size,
        })
        .collect()
}
