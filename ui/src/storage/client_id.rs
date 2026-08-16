pub const CLIENT_ID_STORAGE_KEY: &str = "client-id";

pub fn get() -> String {
    dioxus_utils::js::GlobalAppSettings::get_local_storage()
        .get(CLIENT_ID_STORAGE_KEY)
        .unwrap_or_default()
}

pub fn set(client_id: &str) {
    dioxus_utils::js::GlobalAppSettings::get_local_storage().set(CLIENT_ID_STORAGE_KEY, client_id);
}
