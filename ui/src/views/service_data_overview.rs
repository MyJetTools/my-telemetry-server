use std::rc::Rc;

use crate::{
    models::*,
    states::{DataState, DialogState, MainState},
    utils::to_base_64,
    AppRoute,
};
use dioxus::prelude::*;

#[component]
pub fn ServiceDataOverview(data: Rc<String>) -> Element {
    let mut main_state = consume_context::<Signal<MainState>>();
    let main_state_read_access = main_state.read();

    let hour_key = main_state_read_access.get_hour_key();

    let service_id = main_state_read_access.get_selected_service().unwrap();

    let service_data = match &main_state_read_access.server_data_overview {
        DataState::None => {
            spawn(async move {
                main_state.write().server_data_overview = DataState::Loading;

                let Some(hour_key) = hour_key else {
                    main_state.write().server_data_overview =
                        DataState::Loaded(Rc::new(Vec::new()));
                    return;
                };

                let service_id = service_id.to_string();
                let service_data = data.to_string();
                let client_id = crate::storage::client_id::get();
                let seconds_from = crate::storage::from_time::get();

                let loaded = crate::api::get_by_service_data(
                    hour_key,
                    service_id,
                    service_data,
                    client_id,
                    seconds_from.to_seconds_within_hour(),
                )
                .await;

                match loaded {
                    Ok(mut loaded) => {
                        // Newest first.
                        loaded.sort_by(|i1, i2| i2.started.cmp(&i1.started));

                        main_state.write().server_data_overview =
                            DataState::Loaded(Rc::new(loaded));
                    }
                    Err(err) => {
                        main_state.write().server_data_overview =
                            DataState::Error(err.to_string());
                    }
                }
            });

            return rsx! { "Loading..." };
        }
        DataState::Loading => return rsx! { "Loading..." },
        DataState::Loaded(data) => data,
        DataState::Error(err) => {
            return rsx! {
                div {
                    h1 { "Error" }
                    p { {err.as_str()} }
                }
            }
        }
    };

    let max_duration = get_max(service_data);
    let items = service_data.iter().map(|service_data| {
        let started = service_data.get_started().to_rfc3339();
        let started = &started[..26];

        let duration = format!("{:?}", service_data.get_duration());

        let bar_duration = (service_data.duration as f64 / max_duration) * 100.0;

        let (message, color) = match &service_data.success {
            Some(success) => (success.as_str(), "green"),
            None => match &service_data.error {
                Some(error) => (error.as_str(), "red"),
                None => ("", "black"),
            },
        };

        let tags = service_data.tags.iter().map(|tag| {
            let key = Rc::new(tag.key.to_string());
            let key_show_dialog = key.clone();
            let value = Rc::new(tag.value.to_string());
            let value_show_dialog = value.clone();

            let value = if tag.value.len() > 40 {
                rsx! {
                    span {
                        button {
                            class: "btn btn-sm btn-primary",
                            onclick: move |_| {
                                consume_context::<Signal<MainState>>()
                                    .write()
                                    .show_dialog(DialogState::ShowKeyValue {
                                        the_key: key_show_dialog.clone(),
                                        value: value_show_dialog.clone(),
                                    });
                            },
                            "Show value"
                        }
                    }
                }
            } else {
                rsx! {
                    span { style: "color:black", {tag.value.as_str()} }
                }
            };
            rsx! {
                div { style: "padding:0; color:gray;",
                    " {key.as_str()}: "
                    {value}
                }
            }
        });

        // `id` on this contract is the process id the event belongs to.
        let process_id = service_data.id;

        let service_id_1 = main_state.read().get_selected_service().unwrap();
        let service_id_2 = service_id_1.clone();
        let action_base_64 = to_base_64(data.as_str());

        let data_cloned = data.clone();

        rsx! {
            tr { class: "table-line",
                td {
                    {started},
                    div { style: "width:100%;padding:0",
                        div { style: "width: {bar_duration}%; height: 2px; background-color:green" }
                    }
                }
                td { {duration} }
                td { style: "color: {color}", {message} }
                td { {tags} }
                td {
                    button {
                        class: "btn btn-sm btn-primary",
                        style: "padding: 2px 5px;",
                        Link {
                            onclick: move |_| {
                                consume_context::<Signal<MainState>>()
                                    .write()
                                    .set_show_process(service_id_1.clone(), data_cloned.clone(), process_id);
                            },
                            to: AppRoute::Process {
                                service: service_id_2.to_string(),
                                action: action_base_64,
                                id: process_id,
                            },
                            "Show"
                        }
                    }
                }
            }
        }
    });

    rsx! {
        div { class: "table_top_label",
            b { "{data}" }
            hr {}
        }
        table { class: "table", style: "text-align: left;",
            tr {
                th { "Started" }
                th { "Duration" }
                th { "Message" }
                th { "Tags" }
                th {}
            }
            {items}
        }
    }
}

fn get_max(services: &[MetricHttpModel]) -> f64 {
    let mut result = 0;

    for srv in services {
        if srv.duration > result {
            result = srv.duration;
        }
    }

    result as f64
}
