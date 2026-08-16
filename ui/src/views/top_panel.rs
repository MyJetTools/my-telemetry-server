use dioxus::prelude::*;

use crate::{components::*, states::MainState};

#[component]
pub fn RenderTopPanel() -> Element {
    let mut main_state = consume_context::<Signal<MainState>>();

    let main_state_read_access = main_state.read();

    rsx! {
        table {
            tr {
                td { SelectHourKey {} }
                td { style: "padding-left:15px", "client_id:" }
                td {
                    input {
                        class: "form-control-sm",
                        style: "width: 300px; border: 1px solid lightgray;border-radius: 3px 0 0 3px;",
                        oninput: move |cx| {
                            main_state.write().client_id = cx.value().trim().to_string();
                        },
                        value: main_state_read_access.client_id.as_str()
                    }
                }

                td { style: "padding-left:15px", "From minute and second" }

                td {
                    select {
                        class: "form-control-sm",
                        style: "border: 1px solid lightgray;",
                        oninput: move |cx| {
                            main_state.write().from_time.min = cx.value().parse().unwrap_or_default();
                        },
                        {populate_hours_mins(main_state_read_access.from_time.min)}
                    }
                    ":"
                    select {
                        class: "form-control-sm",
                        style: "border: 1px solid lightgray;",
                        oninput: move |cx| {
                            main_state.write().from_time.sec = cx.value().parse().unwrap_or_default();
                        },
                        {populate_hours_mins(main_state_read_access.from_time.sec)}
                    }
                }
                td { style: "padding-left:15px",
                    button {
                        class: "btn btn-sm btn-primary",
                        onclick: move |_| {
                            main_state.write().apply_client_id();
                        },
                        "Apply"
                    }
                }

                td {}

                td {
                    button {
                        class: "btn btn-sm btn-warning",
                        onclick: move |_| {
                            main_state.write().reset_client_id();
                        },
                        "Reset"
                    }
                }
            }
        }
    }
}

fn populate_hours_mins(value: i64) -> Element {
    let items = (0..60).into_iter().map(|i| {
        let s = format!("{:02}", i);
        rsx! {
            option { selected: value == i, value: i, {s.to_string()} }
        }
    });

    rsx! {
        {items}
    }
}
