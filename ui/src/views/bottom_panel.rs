use dioxus::prelude::*;

use crate::models::TechMetricsHttpModel;
use crate::states::DataState;

#[component]
pub fn RenderBottomPanel() -> Element {
    let mut component_state = use_signal(|| DataState::<TechMetricsHttpModel>::None);

    let component_state_read_access = component_state.read();

    let data = match component_state_read_access.as_ref() {
        DataState::None => {
            spawn(async move {
                match crate::api::get_tech_metrics().await {
                    Ok(data) => {
                        component_state.set(DataState::Loaded(data));
                    }
                    Err(err) => {
                        component_state.set(DataState::Error(err.to_string()));
                    }
                }
            });
            return rsx! { "Loading..." };
        }
        DataState::Loading => {
            return rsx! { "Loading..." };
        }
        DataState::Loaded(data) => data,
        DataState::Error(err) => {
            return rsx! { "{err.as_str()}" };
        }
    };

    rsx! {
        table {
            tr {

                td { "Q by process:" }
                td {
                    {data.queue_by_process_size.to_string()},
                    "/"
                    {data.queue_by_process_capacity.to_string()}
                }

                td {
                    div { class: "separator" }
                }

                td { "AppData Size:" }
                td {
                    {data.app_data_size.to_string()},
                    "/"
                    {data.app_data_capacity.to_string()}
                }

                td {
                    div { class: "separator" }
                }

                td { "AppData Q hours size:" }
                td { {data.app_data_hours_size.to_string()} }
                td {
                    div { class: "separator" }
                }

                td { "AppData Q hours to persist:" }
                td { {data.app_data_to_persist_hours_size.to_string()} }
                td {
                    div { class: "separator" }
                }
                td { "Q size:" }
                td {
                    {data.queue_size.to_string()},
                    "/"
                    {data.queue_capacity.to_string()}
                }
                td {
                    div { class: "separator" }
                }

                td { "UserId links size:" }
                td {
                    {data.user_id_links_size.to_string()},
                    "/"
                    {data.user_id_links_capacity.to_string()}
                }
            }
        }
    }
}
