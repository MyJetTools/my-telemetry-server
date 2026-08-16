use rust_extensions::date_time::{HourKey, IntervalKey};

use crate::app_ctx::AppContext;

pub async fn gc_files(app: &AppContext, from_hour_key: IntervalKey<HourKey>) {
    let files = super::get_metrics_files(app).await;

    for file in files {
        if file.get_hour_key() > from_hour_key {
            continue;
        }

        // Closing has to finish before anything is removed, or the handles outlive the
        // files they point at.
        app.repo.gc(file.get_hour_key()).await;

        let path = file.get_path_and_file_name().to_string();
        let is_folder = file.is_folder();

        tokio::spawn(async move {
            // An hour is a folder, so the data file and every index go together - there is
            // no way to delete half of one.
            let result = if is_folder {
                tokio::fs::remove_dir_all(path.as_str()).await
            } else {
                tokio::fs::remove_file(path.as_str()).await
            };

            if let Err(err) = result {
                println!("Error deleting {}. Err: {:?}", path, err);
            } else {
                println!("{} is deleted", path);
            }
        });
    }
}
