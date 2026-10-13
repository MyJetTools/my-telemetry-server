use crate::app_ctx::AppContext;

pub async fn add_permanent_user(app: &AppContext, user_id: String) {
    let to_save = {
        let mut write_access = app.cache.lock().await;
        write_access
            .permanent_users_list
            .add_permanent_user(user_id)
    };

    crate::scripts::permanent_users::save(app, to_save).await;
}
