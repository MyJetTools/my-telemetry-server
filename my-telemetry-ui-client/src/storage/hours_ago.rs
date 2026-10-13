pub const HOURS_AGO_STORAGE_KEY: &str = "hours-ago";

pub fn get() -> i64 {
    let result = dioxus_utils::js::GlobalAppSettings::get_local_storage()
        .get(HOURS_AGO_STORAGE_KEY)
        .unwrap_or_default();

    result.parse().unwrap_or_default()
}

pub fn set(hours_ago: i64) {
    dioxus_utils::js::GlobalAppSettings::get_local_storage()
        .set(HOURS_AGO_STORAGE_KEY, hours_ago.to_string().as_str());
}
