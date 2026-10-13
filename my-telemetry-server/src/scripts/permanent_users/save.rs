use crate::{app_ctx::AppContext, permanent_users::PermanentUserPersistModel};

use super::PERMANENT_USERS_FILE;

pub async fn save(app: &AppContext, users: Vec<PermanentUserPersistModel>) {
    let content = serde_json::to_string(&users).unwrap();

    let file = app
        .settings_reader
        .get_db_file_prefix(PERMANENT_USERS_FILE)
        .await;

    tokio::fs::write(file, content).await.unwrap();
}
