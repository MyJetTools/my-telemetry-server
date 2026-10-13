use std::{
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use rust_extensions::{date_time::DateTimeAsMicroseconds, MyTimerTick, RepeatTimerIteration};

use crate::app_ctx::AppContext;

/// The hours of S3 older than `HoursToGc` are looked for this often
const S3_GC_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Uploads the hours whose time on the local disk is over, and deletes the hours of S3
/// older than `HoursToGc`
pub struct UploadToS3Timer {
    app: Arc<AppContext>,
    last_s3_gc: AtomicI64,
}

impl UploadToS3Timer {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self {
            app,
            last_s3_gc: AtomicI64::new(0),
        }
    }
}

#[async_trait::async_trait]
impl MyTimerTick for UploadToS3Timer {
    async fn tick(&self) -> RepeatTimerIteration {
        let now = DateTimeAsMicroseconds::now();
        let hours_to_gc = self.app.settings_reader.get_hours_to_gc().await;

        let storage = self.app.repo.clone();

        // The folders of the hours are listed: off the workers of the runtime
        let to_upload =
            tokio::task::spawn_blocking(move || storage.hours_to_upload(now, hours_to_gc))
                .await
                .unwrap();

        for (scope, hour_start) in to_upload {
            match self.app.repo.upload_hour(scope.clone(), hour_start).await {
                Ok(message) => println!("{}", message),
                Err(err) => println!(
                    "S3: {}/{} is not uploaded: {}",
                    scope,
                    crate::metrics_storage::hour_key_of(hour_start),
                    err
                ),
            }
        }

        let last_s3_gc = self.last_s3_gc.load(Ordering::Relaxed);

        if now.unix_microseconds - last_s3_gc >= S3_GC_INTERVAL.as_micros() as i64 {
            self.last_s3_gc
                .store(now.unix_microseconds, Ordering::Relaxed);

            match self.app.repo.gc_s3(now, hours_to_gc).await {
                Ok(hours) => {
                    for hour in hours {
                        println!("S3 GC: {} is deleted", hour);
                    }
                }
                Err(err) => println!("S3 GC failed: {}", err),
            }
        }

        RepeatTimerIteration::WithInterval
    }
}
