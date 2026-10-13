use dioxus::prelude::*;

use crate::{states::MainState, AppRoute};

/// Which my-telemetry-server the UI reads.
#[component]
pub fn SelectEnv() -> Element {
    let mut main_state = consume_context::<Signal<MainState>>();

    let read_access = main_state.read();

    let selected = read_access.get_env();

    let options = read_access.envs.iter().flatten().map(|env| {
        let is_selected = selected
            .as_ref()
            .map_or(false, |selected| selected.as_str() == env.as_str());

        rsx! {
            option { selected: is_selected, value: env.as_str(), {env.as_str()} }
        }
    });

    rsx! {
        select {
            class: "form-select-sm",
            style: "border-color: lightgray;",
            oninput: move |e| {
                let value = e.value();
                main_state.write().set_env(value.as_str());
                // A service or a process selected in the route belongs to the env before
                navigator().push(AppRoute::Home);
            },
            {options}
        }
    }
}
