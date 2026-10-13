use crate::{app_ctx::AppContext, permanent_users::PermanentUserPersistModel};

pub async fn get_permanent_users(app: &AppContext) -> Vec<PermanentUserPersistModel> {
    let read_access = app.cache.lock().await;
    read_access.permanent_users_list.get_all()
}
