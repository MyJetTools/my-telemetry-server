pub const SCOPE_STORAGE_KEY: &str = "scope";

pub fn get() -> String {
    dioxus_utils::js::GlobalAppSettings::get_local_storage()
        .get(SCOPE_STORAGE_KEY)
        .unwrap_or_default()
}

pub fn set(scope: &str) {
    dioxus_utils::js::GlobalAppSettings::get_local_storage().set(SCOPE_STORAGE_KEY, scope);
}
