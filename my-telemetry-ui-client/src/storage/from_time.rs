use crate::models::TimeModel;

pub const FROM_TIME_KEY: &str = "from-time";

pub fn get() -> TimeModel {
    let result = dioxus_utils::js::GlobalAppSettings::get_local_storage()
        .get(FROM_TIME_KEY)
        .unwrap_or_default();

    TimeModel::from_str(result.as_str())
}

pub fn set(time_model: &TimeModel) {
    dioxus_utils::js::GlobalAppSettings::get_local_storage()
        .set(FROM_TIME_KEY, time_model.to_string().as_str());
}
