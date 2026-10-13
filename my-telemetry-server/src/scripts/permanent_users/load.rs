use crate::app_ctx::AppContext;

use super::PERMANENT_USERS_FILE;

pub async fn load(app: &AppContext) -> Vec<String> {
    let file_name = app
        .settings_reader
        .get_db_file_prefix(PERMANENT_USERS_FILE)
        .await;

    let files = tokio::fs::read(file_name.as_str()).await;
    if files.is_err() {
        println!(
            "No permanent users file found. Skipping loading permanent users. {}",
            file_name
        );
        return vec![];
    }

    let files = files.unwrap();

    serde_json::from_slice(files.as_slice()).unwrap()
}
